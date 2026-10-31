use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use sanchoris_config::validate::covers;
use sanchoris_config::{PermissionLevel, Session};
use sanchoris_core::channel::{OutboxType, enqueue_outbox};
use sanchoris_core::conversations::post_agent_message;
use sanchoris_core::db::{REF_CHECK_WAIT, REF_INVOCATION, TenantTx, insert_ref, locate};
use sanchoris_core::item::{Entity, Key, format_time};
use sanchoris_core::items::{
    ApprovalWait, CheckWait, CheckWaitMarker, FiledIssue, Invocation as InvocationItem,
    OUTPUT_COMMENT, OUTPUT_ISSUE, OUTPUT_PULL_REQUEST, Output, TimerItem,
};
use sanchoris_core::jobs::Job;
use sanchoris_core::keys;
use sanchoris_core::model::TaskStatus;
use sanchoris_core::skillrun::{SkillRunError, apply_event, lock_skill_run, skill_run_item};
use sanchoris_core::store::{Index, SortKey};
use sanchoris_core::tasks::{ERROR_APPROVAL_REQUESTED, load_task, store_task, task_branch};
use sanchoris_engine::{Disposition, Event, IgnoreReason, Invocation, ResourceOutcome, Timer};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::error::{Result, WorkerError};
use crate::ports::{CheckRun, InvocationContext, repository_named};
use crate::resources::param_repos;
use crate::{Worker, installation_for, installations};

const LOG_LIMIT: usize = 4000;
pub const ERROR_CHAT_UNAVAILABLE: &str = "Chat.Unavailable";
pub const ERROR_FORBIDDEN: &str = "Resource.Forbidden";
const PASSING: [&str; 3] = ["success", "neutral", "skipped"];

pub async fn fire_timer(
    worker: &Worker,
    tenant_id: Uuid,
    task_id: Uuid,
    skill_run_id: Uuid,
    timer_id: Uuid,
    message_id: Uuid,
) -> Result<usize> {
    let now = worker.now();
    let mut tx = worker.tx(tenant_id);
    let Some(skill_run) = skill_run_item(&mut tx, skill_run_id).await? else {
        return Ok(0);
    };
    let key = keys::timer_key(tenant_id, task_id, skill_run.number, timer_id);
    let Some(item) = tx
        .get::<TimerItem>(&key)
        .await?
        .filter(|t| t.message_id == message_id)
    else {
        return Ok(0);
    };
    let timer: Timer = serde_json::from_value(item.timer.clone())?;
    let disposition = if load_task(&mut tx, task_id).await?.status.is_terminal() {
        None
    } else {
        match apply_event(&mut tx, skill_run_id, Event::TimerDue { timer }, now).await {
            Ok(disposition) => Some(disposition),
            Err(SkillRunError::NotFound(_)) => None,
            Err(error) => return Err(error.into()),
        }
    };
    if let Some(mut item) = tx
        .get::<TimerItem>(&key)
        .await?
        .filter(|t| t.message_id == message_id)
    {
        if disposition == Some(Disposition::Ignored(IgnoreReason::TimerNotDue)) {
            item.message_id = tx.emit_at(
                Job::EngineTimer {
                    tenant_id,
                    task_id,
                    skill_run_id,
                    timer_id,
                },
                item.due_at,
            );
            tx.put(&item)?;
        } else {
            tx.delete(&key)?;
        }
    }
    worker.faults.check("timer.before_commit")?;
    tx.commit().await?;
    Ok(1)
}

pub async fn invoke(worker: &Worker, tenant_id: Uuid, invocation_id: Uuid) -> Result<usize> {
    let mut tx = worker.tx(tenant_id);
    let Some(location) = locate(&mut tx, invocation_id, REF_INVOCATION).await? else {
        return Ok(0);
    };
    let Some(mut item) = tx
        .get::<InvocationItem>(&location.target)
        .await?
        .filter(|i| i.status == "pending")
    else {
        return Ok(0);
    };
    item.attempts += 1;
    let Some(context) = invocation_context(&mut tx, item.skill_run_id).await? else {
        item.status = "done".into();
        item.completed_at = Some(worker.now());
        tx.put(&item)?;
        tx.commit().await?;
        return Ok(1);
    };
    tx.put(&item)?;
    let invocation: Invocation = serde_json::from_value(item.invocation.clone())?;
    match invoke_resource(worker, &mut tx, &context, &item, &invocation).await? {
        Some(outcome) => complete_invocation(worker, &mut tx, &location.target, outcome).await?,
        None => {
            let mut item = tx.require::<InvocationItem>(&location.target).await?;
            item.status = "awaiting".into();
            tx.put(&item)?;
        }
    }
    worker.faults.check("invocation.before_commit")?;
    tx.commit().await?;
    Ok(1)
}

pub(crate) async fn complete_invocation(
    worker: &Worker,
    tx: &mut TenantTx,
    key: &Key,
    outcome: ResourceOutcome,
) -> Result<()> {
    let mut item = tx.require::<InvocationItem>(key).await?;
    if item.status == "done" {
        return Ok(());
    }
    item.status = "done".into();
    item.result = Some(serde_json::to_value(&outcome)?);
    item.completed_at = Some(worker.now());
    tx.put(&item)?;
    let invocation: Invocation = serde_json::from_value(item.invocation.clone())?;
    if let ResourceOutcome::Succeeded { output } = &outcome {
        record_outputs(tx, &item, &invocation, output, worker.now()).await?;
    }
    apply_event(
        tx,
        item.skill_run_id,
        Event::ResourceResult {
            key: invocation.idempotency_key,
            outcome,
        },
        worker.now(),
    )
    .await?;
    Ok(())
}

pub(crate) async fn invocation_by_id(
    tx: &mut TenantTx,
    invocation_id: Uuid,
) -> Result<Option<(Key, InvocationItem)>> {
    let Some(location) = locate(tx, invocation_id, REF_INVOCATION).await? else {
        return Ok(None);
    };
    Ok(tx
        .get::<InvocationItem>(&location.target)
        .await?
        .map(|item| (location.target, item)))
}

async fn record_outputs(
    tx: &mut TenantTx,
    item: &InvocationItem,
    invocation: &Invocation,
    output: &Value,
    now: DateTime<Utc>,
) -> Result<()> {
    let items = match invocation.resource.name().as_str() {
        "github:createPullRequests" => output
            .as_array()
            .map(|prs| {
                prs.iter()
                    .map(|pr| {
                        (
                            OUTPUT_PULL_REQUEST,
                            pr["Repo"].clone(),
                            pr["Number"].clone(),
                            pr["Url"].clone(),
                            None,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        "github:commentOnIssue" => vec![(
            OUTPUT_COMMENT,
            output["Repo"].clone(),
            output["Number"].clone(),
            output["Url"].clone(),
            None,
        )],
        "github:createIssue" => vec![(
            OUTPUT_ISSUE,
            output["Repo"].clone(),
            output["Number"].clone(),
            output["Url"].clone(),
            output["Title"].as_str().map(str::to_string),
        )],
        _ => return Ok(()),
    };
    if items.is_empty() {
        return Ok(());
    }
    let installations = installations(tx).await?;
    for (kind, repo, number, url, title) in items {
        let (Some(repo), Some(number), Some(url)) = (repo.as_str(), number.as_i64(), url.as_str())
        else {
            continue;
        };
        let repository_id = repository_named(&installations, repo)
            .or_else(|| repository_named(&installations, &url_repository(url)?))
            .map(|repository| repository.id)
            .ok_or_else(|| {
                WorkerError::Invalid(format!(
                    "{repo} is not a repository of the tenant's GitHub App installation"
                ))
            })?;
        let output = Output {
            id: Uuid::new_v4(),
            tenant_id: tx.tenant_id(),
            task_id: item.task_id,
            skill_run_id: item.skill_run_id,
            skill_run_number: item.skill_run_number,
            kind: kind.to_string(),
            repository: repo.to_string(),
            repository_id,
            number,
            url: url.to_string(),
            title,
            state: (kind == OUTPUT_PULL_REQUEST).then(|| "open".to_string()),
            created_at: now,
            updated_at: now,
        };
        if tx.get::<Output>(&output.key()).await?.is_none() {
            tx.insert(&output)?;
        }
        if kind == OUTPUT_ISSUE {
            let filed = FiledIssue {
                tenant_id: tx.tenant_id(),
                repository_id,
                number,
                task_id: item.task_id,
                created_at: now,
            };
            if tx.get::<FiledIssue>(&filed.key()).await?.is_none() {
                tx.insert(&filed)?;
            }
        }
    }
    Ok(())
}

fn url_repository(url: &str) -> Option<String> {
    let path = url.strip_prefix("https://github.com/")?;
    let mut parts = path.split('/');
    Some(format!("{}/{}", parts.next()?, parts.next()?))
}

async fn invocation_context(
    tx: &mut TenantTx,
    skill_run_id: Uuid,
) -> Result<Option<InvocationContext>> {
    let locked = lock_skill_run(tx, skill_run_id).await?;
    if locked.run.status.is_finished() {
        return Ok(None);
    }
    let task = load_task(tx, locked.meta.task_id).await?;
    if task.status.is_terminal() {
        return Ok(None);
    }
    let installations = installations(tx).await?;
    let repositories = installations
        .iter()
        .flat_map(|installation| installation.repositories.clone())
        .collect::<Vec<_>>();
    Ok(Some(InvocationContext {
        tenant_id: tx.tenant_id(),
        task_id: locked.meta.task_id,
        task_name: task.display_name(&repositories),
        task_title: task.title,
        skill_run_id,
        skill_run_number: locked.meta.number,
        state_name: locked
            .run
            .current
            .as_ref()
            .map(|c| c.state_name.clone())
            .unwrap_or_default(),
        branch: task_branch(locked.meta.task_id),
        installations,
    }))
}

fn failed(error: &str, cause: impl Into<String>) -> ResourceOutcome {
    ResourceOutcome::Failed {
        error: Some(error.to_string()),
        cause: Some(cause.into()),
    }
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

pub fn push_commands(repos: &[String], branch: &str) -> Vec<String> {
    repos
        .iter()
        .map(|repo| {
            let dir = repo.rsplit('/').next().expect("owner/name");
            format!(
                "git -C {} push origin {}",
                shell_quote(dir),
                shell_quote(&format!("HEAD:refs/heads/{branch}"))
            )
        })
        .collect()
}

async fn run_of_skill(tx: &mut TenantTx, skill_run_id: Uuid) -> Result<Option<Uuid>> {
    Ok(skill_run_item(tx, skill_run_id)
        .await?
        .and_then(|item| item.run_id))
}

async fn invoke_resource(
    worker: &Worker,
    tx: &mut TenantTx,
    context: &InvocationContext,
    item: &InvocationItem,
    invocation: &Invocation,
) -> Result<Option<ResourceOutcome>> {
    let tenant_id = tx.tenant_id();
    let parameters = &invocation.parameters;
    let name = invocation.resource.name();
    match name.as_str() {
        "git:push" | "workspace:exec" => {
            let commands = if name == "git:push" {
                match param_repos(parameters) {
                    Ok(repos) => push_commands(&repos, &context.branch),
                    Err(cause) => return Ok(Some(failed("Resource.InvalidParameters", cause))),
                }
            } else {
                match parameters.get("Commands").and_then(Value::as_array) {
                    Some(items) if items.iter().all(Value::is_string) => items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect(),
                    _ => {
                        return Ok(Some(failed(
                            "Resource.InvalidParameters",
                            "Commands must be an array of strings",
                        )));
                    }
                }
            };
            let Some(run_id) = run_of_skill(tx, item.skill_run_id).await? else {
                return Ok(Some(failed(
                    "Workspace.Unavailable",
                    "the skill run has no session to run commands in",
                )));
            };
            enqueue_outbox(
                tx,
                run_id,
                OutboxType::Exec,
                &json!({
                    "requestId": item.id,
                    "purpose": if name == "git:push" { "git_push" } else { "exec" },
                    "commands": commands,
                }),
                &format!("exec:{}", item.id),
            )
            .await?;
            Ok(None)
        }
        "github:checks" => {
            let repos = match param_repos(parameters) {
                Ok(repos) => repos,
                Err(cause) => return Ok(Some(failed("Resource.InvalidParameters", cause))),
            };
            let token = invocation
                .task_token
                .clone()
                .ok_or_else(|| WorkerError::Invalid("checks needs a task token".into()))?;
            let repository_ids = match repos
                .iter()
                .map(|repo| {
                    repository_named(&context.installations, repo)
                        .map(|repository| repository.id)
                        .ok_or_else(|| {
                            format!(
                                "{repo} is not a repository of the tenant's GitHub App installation"
                            )
                        })
                })
                .collect::<std::result::Result<Vec<_>, _>>()
            {
                Ok(ids) => ids,
                Err(cause) => return Ok(Some(failed("Resource.InvalidParameters", cause))),
            };
            let key = keys::check_wait_key(tenant_id, item.task_id, item.skill_run_number, &token);
            if tx.get::<CheckWait>(&key).await?.is_none() {
                let id = Uuid::new_v4();
                tx.insert(&CheckWait {
                    id,
                    tenant_id,
                    task_id: item.task_id,
                    skill_run_id: item.skill_run_id,
                    skill_run_number: item.skill_run_number,
                    task_token: token,
                    repos: repos.clone(),
                    head_shas: json!({}),
                    status: "waiting".into(),
                    next_poll_at: worker.now(),
                    created_at: worker.now(),
                    completed_at: None,
                })?;
                tx.insert(&CheckWaitMarker {
                    tenant_id,
                    wait_id: id,
                    task_id: item.task_id,
                    repos: repos.clone(),
                    repository_ids,
                })?;
                insert_ref(tx, id, REF_CHECK_WAIT, key, Some(item.task_id))?;
                tx.emit(Job::CheckPoll {
                    tenant_id,
                    task_id: item.task_id,
                    wait_id: id,
                });
            }
            Ok(Some(ResourceOutcome::Succeeded {
                output: json!({ "Watching": repos }),
            }))
        }
        "chat:post" => {
            let Some(text) = parameters.get("Text").and_then(Value::as_str) else {
                return Ok(Some(failed(
                    "Resource.InvalidParameters",
                    "Text must be a string",
                )));
            };
            let task = load_task(tx, context.task_id).await?;
            if task.origin.conversation_id().is_none() {
                return Ok(Some(failed(
                    ERROR_CHAT_UNAVAILABLE,
                    "the task has no conversation to post to",
                )));
            }
            let run_id = run_of_skill(tx, item.skill_run_id).await?;
            post_agent_message(tx, &task, run_id, item.id, text, item.created_at).await?;
            Ok(Some(ResourceOutcome::Succeeded {
                output: json!({ "Posted": true }),
            }))
        }
        "github:createIssue" => {
            let task = load_task(tx, context.task_id).await?;
            let session: Session =
                serde_json::from_value(task.definition["Session"]["Definition"].clone())?;
            let Some(repo) = parameters.get("Repo").and_then(Value::as_str) else {
                return Ok(Some(failed(
                    "Resource.InvalidParameters",
                    "Repo must be owner/name",
                )));
            };
            if !covers(&session.github.repos, repo) {
                return Ok(Some(failed(
                    ERROR_FORBIDDEN,
                    format!("{repo} is outside the session's GitHub.Repos"),
                )));
            }
            if session
                .github
                .permissions
                .get("issues")
                .is_none_or(|level| *level < PermissionLevel::Write)
            {
                return Ok(Some(failed(
                    ERROR_FORBIDDEN,
                    "the session does not grant issues: write",
                )));
            }
            Ok(Some(
                worker.deps.resources.invoke(context, invocation).await,
            ))
        }
        "approval:request" => {
            let Some(message) = parameters.get("Message").and_then(Value::as_str) else {
                return Ok(Some(failed(
                    "Resource.InvalidParameters",
                    "Message must be a string",
                )));
            };
            let token = invocation
                .task_token
                .clone()
                .ok_or_else(|| WorkerError::Invalid("approval needs a task token".into()))?;
            let key =
                keys::approval_wait_key(tenant_id, item.task_id, item.skill_run_number, &token);
            if tx.get::<ApprovalWait>(&key).await?.is_none() {
                tx.insert(&ApprovalWait {
                    id: Uuid::new_v4(),
                    tenant_id,
                    task_id: context.task_id,
                    skill_run_id: item.skill_run_id,
                    skill_run_number: item.skill_run_number,
                    task_token: token,
                    message: message.to_string(),
                    status: "waiting".into(),
                    created_at: worker.now(),
                    resolved_at: None,
                })?;
            }
            let mut task = load_task(tx, context.task_id).await?;
            if matches!(task.status, TaskStatus::Running | TaskStatus::Waiting) {
                task.status = TaskStatus::NeedsYou;
                task.wait_reason = None;
                task.resume_at = None;
                task.error = Some(ERROR_APPROVAL_REQUESTED.into());
                task.cause = Some(message.to_string());
                task.updated_at = worker.now();
                store_task(tx, &task, worker.now()).await?;
            }
            Ok(Some(ResourceOutcome::Succeeded {
                output: json!({ "Requested": true }),
            }))
        }
        _ => Ok(Some(
            worker.deps.resources.invoke(context, invocation).await,
        )),
    }
}

enum Verdict {
    Pending,
    Done(Value),
}

pub async fn sweep_checks(worker: &Worker) -> Result<usize> {
    let now = worker.now();
    let due = worker
        .store
        .query_index::<CheckWait>(
            Index::Gsi2,
            keys::SWEEP_CHECKS,
            SortKey::AtMost(format!("{}#~", format_time(now))),
        )
        .await?;
    for wait in &due {
        let mut tx = worker.tx(wait.tenant_id);
        tx.emit(Job::CheckPoll {
            tenant_id: wait.tenant_id,
            task_id: wait.task_id,
            wait_id: wait.id,
        });
        tx.commit().await?;
    }
    Ok(due.len())
}

pub async fn poll_check(worker: &Worker, tenant_id: Uuid, wait_id: Uuid) -> Result<usize> {
    let now = worker.now();
    let mut tx = worker.tx(tenant_id);
    let Some(location) = locate(&mut tx, wait_id, REF_CHECK_WAIT).await? else {
        return Ok(0);
    };
    let Some(mut wait) = tx
        .get::<CheckWait>(&location.target)
        .await?
        .filter(|w| w.status == "waiting" && w.next_poll_at <= now)
    else {
        return Ok(0);
    };
    let (verdict, head_shas) = poll(worker, &mut tx, &wait, &task_branch(wait.task_id)).await?;
    wait.head_shas = json!(head_shas);
    match verdict {
        Verdict::Pending => {
            wait.next_poll_at = now + worker.settings.check_poll_interval;
            tx.put(&wait)?;
        }
        Verdict::Done(output) => {
            wait.status = "done".into();
            wait.completed_at = Some(now);
            tx.put(&wait)?;
            tx.delete(&keys::check_wait_marker_key(tenant_id, wait.id))?;
            apply_event(
                &mut tx,
                wait.skill_run_id,
                Event::TaskSuccess {
                    task_token: wait.task_token.clone(),
                    output,
                },
                now,
            )
            .await?;
        }
    }
    worker.faults.check("checks.before_commit")?;
    tx.commit().await?;
    Ok(1)
}

fn excerpt(run: &CheckRun) -> String {
    let mut text = format!(
        "{}: {}",
        run.name,
        run.conclusion.as_deref().unwrap_or("none")
    );
    for part in [&run.summary, &run.text].into_iter().flatten() {
        text.push('\n');
        text.push_str(part);
    }
    text
}

async fn poll(
    worker: &Worker,
    tx: &mut TenantTx,
    wait: &CheckWait,
    branch: &str,
) -> Result<(Verdict, BTreeMap<String, String>)> {
    let installations = installations(tx).await?;
    let mut head_shas = BTreeMap::new();
    let mut pull_requests = Vec::new();
    let mut failed_checks = Vec::new();
    let mut logs = Vec::new();
    let mut pending = false;
    for repo in &wait.repos {
        let installation = installation_for(&installations, repo)?;
        let Some(pr) = worker
            .deps
            .github
            .find_open_pull_request(installation, repo, branch)
            .await?
        else {
            pending = true;
            continue;
        };
        head_shas.insert(repo.clone(), pr.head_sha.clone());
        let runs = worker
            .deps
            .github
            .list_check_runs(installation, repo, &pr.head_sha)
            .await?;
        if runs.is_empty() || runs.iter().any(|r| r.status != "completed") {
            pending = true;
            continue;
        }
        pull_requests.push(json!({ "Repo": repo, "Number": pr.number, "Sha": pr.head_sha }));
        let failing = runs
            .iter()
            .filter(|r| !PASSING.contains(&r.conclusion.as_deref().unwrap_or("")))
            .collect::<Vec<_>>();
        if !failing.is_empty() {
            failed_checks.push(format!(
                "{repo}#{} ({})",
                pr.number,
                failing
                    .iter()
                    .map(|r| r.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            logs.extend(failing.iter().map(|r| excerpt(r)));
        }
    }
    if pending {
        return Ok((Verdict::Pending, head_shas));
    }
    let output = if failed_checks.is_empty() {
        json!({ "Conclusion": "success", "PullRequests": pull_requests })
    } else {
        let mut log = logs.join("\n\n");
        if log.len() > LOG_LIMIT {
            let cut = (0..=LOG_LIMIT)
                .rev()
                .find(|i| log.is_char_boundary(*i))
                .unwrap_or(0);
            log.truncate(cut);
        }
        json!({
            "Conclusion": "failure",
            "Failed": failed_checks.join("; "),
            "Log": log,
            "PullRequests": pull_requests,
        })
    };
    Ok((Verdict::Done(output), head_shas))
}
