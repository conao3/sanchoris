use std::collections::BTreeMap;

use sanchoris_core::channel::{OutboxType, enqueue_outbox};
use sanchoris_core::db::{TenantTx, claim_due_jobs, complete_due_job};
use sanchoris_core::skillrun::{DUE_ENGINE_TIMER, SkillRunError, apply_event, lock_skill_run};
use sanchoris_core::tasks::{ERROR_APPROVAL_REQUESTED, task_branch};
use sanchoris_engine::{Event, Invocation, ResourceOutcome, Timer};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::error::{Result, WorkerError};
use crate::ports::{CheckRun, InvocationContext};
use crate::resources::param_repos;
use crate::{Worker, adopt, installation_for, installations};

const LOG_LIMIT: usize = 4000;
const PASSING: [&str; 3] = ["success", "neutral", "skipped"];

pub async fn tick_timers(worker: &Worker) -> Result<usize> {
    let mut processed = 0;
    loop {
        let now = worker.now();
        let mut tx = worker.begin().await?;
        let Some(job) = claim_due_jobs(&mut tx, DUE_ENGINE_TIMER, now, 1)
            .await?
            .into_iter()
            .next()
        else {
            return Ok(processed);
        };
        let row = sqlx::query("SELECT payload FROM sanchoris.due_jobs WHERE id = $1")
            .bind(job.id)
            .fetch_one(&mut *tx)
            .await?;
        let payload: Value = row.try_get("payload")?;
        complete_due_job(&mut tx, job.id).await?;
        let mut tx = adopt(tx, job.tenant_id).await?;
        let skill_run_id: Uuid = serde_json::from_value(payload["skill_run_id"].clone())?;
        let timer: Timer = serde_json::from_value(payload["timer"].clone())?;
        match apply_event(&mut tx, skill_run_id, Event::TimerDue { timer }, now).await {
            Ok(_) | Err(SkillRunError::NotFound(_)) => {}
            Err(error) => return Err(error.into()),
        }
        worker.faults.check("timer.before_commit")?;
        tx.commit().await?;
        processed += 1;
    }
}

struct Claimed {
    id: Uuid,
    skill_run_id: Uuid,
    invocation: Invocation,
}

pub async fn tick_invocations(worker: &Worker) -> Result<usize> {
    let mut processed = 0;
    loop {
        let mut tx = worker.begin().await?;
        let Some(row) = sqlx::query(
            "SELECT id, tenant_id, skill_run_id, invocation FROM sanchoris.resource_invocations \
             WHERE status = 'pending' ORDER BY created_at, id LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(processed);
        };
        let claimed = Claimed {
            id: row.try_get("id")?,
            skill_run_id: row.try_get("skill_run_id")?,
            invocation: serde_json::from_value(row.try_get("invocation")?)?,
        };
        let mut tx = adopt(tx, row.try_get("tenant_id")?).await?;
        sqlx::query(
            "UPDATE sanchoris.resource_invocations SET attempts = attempts + 1 WHERE id = $1",
        )
        .bind(claimed.id)
        .execute(tx.conn())
        .await?;
        let Some(context) = invocation_context(&mut tx, claimed.skill_run_id).await? else {
            sqlx::query(
                "UPDATE sanchoris.resource_invocations SET status = 'done', completed_at = $2 WHERE id = $1",
            )
            .bind(claimed.id)
            .bind(worker.now())
            .execute(tx.conn())
            .await?;
            tx.commit().await?;
            processed += 1;
            continue;
        };
        let outcome = match invoke(worker, &mut tx, &context, &claimed).await? {
            Some(outcome) => outcome,
            None => {
                sqlx::query(
                    "UPDATE sanchoris.resource_invocations SET status = 'awaiting' WHERE id = $1",
                )
                .bind(claimed.id)
                .execute(tx.conn())
                .await?;
                worker.faults.check("invocation.before_commit")?;
                tx.commit().await?;
                processed += 1;
                continue;
            }
        };
        complete_invocation(worker, &mut tx, claimed.id, outcome).await?;
        worker.faults.check("invocation.before_commit")?;
        tx.commit().await?;
        processed += 1;
    }
}

pub(crate) async fn complete_invocation(
    worker: &Worker,
    tx: &mut TenantTx,
    invocation_id: Uuid,
    outcome: ResourceOutcome,
) -> Result<()> {
    let row = sqlx::query(
        "UPDATE sanchoris.resource_invocations SET status = 'done', result = $2, completed_at = $3 \
         WHERE id = $1 AND status <> 'done' RETURNING skill_run_id, invocation",
    )
    .bind(invocation_id)
    .bind(serde_json::to_value(&outcome)?)
    .bind(worker.now())
    .fetch_optional(tx.conn())
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let skill_run_id: Uuid = row.try_get("skill_run_id")?;
    let invocation: Invocation = serde_json::from_value(row.try_get("invocation")?)?;
    if let ResourceOutcome::Succeeded { output } = &outcome {
        record_outputs(tx, skill_run_id, &invocation, output).await?;
    }
    apply_event(
        tx,
        skill_run_id,
        Event::ResourceResult {
            key: invocation.idempotency_key,
            outcome,
        },
        worker.now(),
    )
    .await?;
    Ok(())
}

async fn record_outputs(
    tx: &mut TenantTx,
    skill_run_id: Uuid,
    invocation: &Invocation,
    output: &Value,
) -> Result<()> {
    let items = match invocation.resource.name().as_str() {
        "github:createPullRequests" => output
            .as_array()
            .map(|prs| {
                prs.iter()
                    .map(|pr| {
                        (
                            "pull_request",
                            pr["Repo"].clone(),
                            pr["Number"].clone(),
                            pr["Url"].clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        "github:commentOnIssue" => vec![(
            "comment",
            output["Repo"].clone(),
            output["Number"].clone(),
            output["Url"].clone(),
        )],
        _ => return Ok(()),
    };
    for (kind, repo, number, url) in items {
        let (Some(repo), Some(number), Some(url)) = (repo.as_str(), number.as_i64(), url.as_str())
        else {
            continue;
        };
        sqlx::query(
            "INSERT INTO sanchoris.outputs (tenant_id, skill_run_id, kind, repository, number, url, state) \
             SELECT $1, $2, $3, $4, $5, $6, $7 \
             WHERE NOT EXISTS (SELECT 1 FROM sanchoris.outputs WHERE skill_run_id = $2 AND kind = $3 \
               AND repository = $4 AND number = $5)",
        )
        .bind(tx.tenant_id())
        .bind(skill_run_id)
        .bind(kind)
        .bind(repo)
        .bind(i32::try_from(number).map_err(|_| WorkerError::Invalid("number overflow".into()))?)
        .bind(url)
        .bind((kind == "pull_request").then_some("open"))
        .execute(tx.conn())
        .await?;
    }
    Ok(())
}

async fn invocation_context(
    tx: &mut TenantTx,
    skill_run_id: Uuid,
) -> Result<Option<InvocationContext>> {
    let locked = lock_skill_run(tx, skill_run_id).await?;
    if locked.run.status.is_finished() {
        return Ok(None);
    }
    let task = sqlx::query("SELECT name, title FROM sanchoris.tasks WHERE id = $1")
        .bind(locked.meta.task_id)
        .fetch_one(tx.conn())
        .await?;
    Ok(Some(InvocationContext {
        tenant_id: tx.tenant_id(),
        task_id: locked.meta.task_id,
        task_name: task.try_get("name")?,
        task_title: task.try_get("title")?,
        skill_run_id,
        skill_run_number: locked.meta.number,
        state_name: locked
            .run
            .current
            .as_ref()
            .map(|c| c.state_name.clone())
            .unwrap_or_default(),
        branch: task_branch(locked.meta.task_id),
        installations: installations(tx).await?,
    }))
}

fn failed(error: &str, cause: impl Into<String>) -> ResourceOutcome {
    ResourceOutcome::Failed {
        error: Some(error.to_string()),
        cause: Some(cause.into()),
    }
}

async fn run_of_skill(tx: &mut TenantTx, skill_run_id: Uuid) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT run_id FROM sanchoris.skill_runs WHERE id = $1",
    )
    .bind(skill_run_id)
    .fetch_one(tx.conn())
    .await?)
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

async fn invoke(
    worker: &Worker,
    tx: &mut TenantTx,
    context: &InvocationContext,
    claimed: &Claimed,
) -> Result<Option<ResourceOutcome>> {
    let invocation = &claimed.invocation;
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
            let Some(run_id) = run_of_skill(tx, claimed.skill_run_id).await? else {
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
                    "requestId": claimed.id,
                    "purpose": if name == "git:push" { "git_push" } else { "exec" },
                    "commands": commands,
                }),
                &format!("exec:{}", claimed.id),
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
            sqlx::query(
                "INSERT INTO sanchoris.check_waits (tenant_id, skill_run_id, task_token, repos, status, next_poll_at) \
                 VALUES ($1, $2, $3, $4, 'waiting', $5) ON CONFLICT (skill_run_id, task_token) DO NOTHING",
            )
            .bind(tx.tenant_id())
            .bind(claimed.skill_run_id)
            .bind(&token)
            .bind(json!(repos))
            .bind(worker.now())
            .execute(tx.conn())
            .await?;
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
            let Some(run_id) = run_of_skill(tx, claimed.skill_run_id).await? else {
                return Ok(Some(failed(
                    "Chat.Unavailable",
                    "the skill run has no session",
                )));
            };
            let channel = sqlx::query_scalar::<_, Option<Value>>(
                "SELECT channel FROM sanchoris.tasks WHERE id = $1",
            )
            .bind(context.task_id)
            .fetch_one(tx.conn())
            .await?;
            enqueue_outbox(
                tx,
                run_id,
                OutboxType::ChatPost,
                &json!({ "text": text, "channel": channel }),
                &format!("chat:{}", invocation.idempotency_key),
            )
            .await?;
            Ok(Some(ResourceOutcome::Succeeded {
                output: json!({ "Posted": true }),
            }))
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
            sqlx::query(
                "INSERT INTO sanchoris.approval_waits (tenant_id, task_id, skill_run_id, task_token, message, status) \
                 VALUES ($1, $2, $3, $4, $5, 'waiting') ON CONFLICT (skill_run_id, task_token) DO NOTHING",
            )
            .bind(tx.tenant_id())
            .bind(context.task_id)
            .bind(claimed.skill_run_id)
            .bind(&token)
            .bind(message)
            .execute(tx.conn())
            .await?;
            sqlx::query(
                "UPDATE sanchoris.tasks SET status = 'NEEDS_YOU', wait_reason = NULL, resume_at = NULL, \
                 error = $2, cause = $3, updated_at = $4 \
                 WHERE id = $1 AND status IN ('RUNNING', 'WAITING')",
            )
            .bind(context.task_id)
            .bind(ERROR_APPROVAL_REQUESTED)
            .bind(message)
            .bind(worker.now())
            .execute(tx.conn())
            .await?;
            Ok(Some(ResourceOutcome::Succeeded {
                output: json!({ "Requested": true }),
            }))
        }
        _ => Ok(Some(
            worker.deps.resources.invoke(context, invocation).await,
        )),
    }
}

struct Wait {
    id: Uuid,
    skill_run_id: Uuid,
    task_token: String,
    repos: Vec<String>,
}

enum Verdict {
    Pending,
    Done(Value),
}

pub async fn tick_checks(worker: &Worker) -> Result<usize> {
    let mut processed = 0;
    loop {
        let now = worker.now();
        let mut tx = worker.begin().await?;
        let Some(row) = sqlx::query(
            "SELECT id, tenant_id, skill_run_id, task_token, repos FROM sanchoris.check_waits \
             WHERE status = 'waiting' AND next_poll_at <= $1 ORDER BY next_poll_at, id LIMIT 1 \
             FOR UPDATE SKIP LOCKED",
        )
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(processed);
        };
        let wait = Wait {
            id: row.try_get("id")?,
            skill_run_id: row.try_get("skill_run_id")?,
            task_token: row.try_get("task_token")?,
            repos: serde_json::from_value(row.try_get("repos")?)?,
        };
        let mut tx = adopt(tx, row.try_get("tenant_id")?).await?;
        let task_id =
            sqlx::query_scalar::<_, Uuid>("SELECT task_id FROM sanchoris.skill_runs WHERE id = $1")
                .bind(wait.skill_run_id)
                .fetch_one(tx.conn())
                .await?;
        let (verdict, head_shas) = poll(worker, &mut tx, &wait, &task_branch(task_id)).await?;
        match verdict {
            Verdict::Pending => {
                sqlx::query(
                    "UPDATE sanchoris.check_waits SET head_shas = $2, next_poll_at = $3 WHERE id = $1",
                )
                .bind(wait.id)
                .bind(json!(head_shas))
                .bind(now + worker.settings.check_poll_interval)
                .execute(tx.conn())
                .await?;
            }
            Verdict::Done(output) => {
                sqlx::query(
                    "UPDATE sanchoris.check_waits SET head_shas = $2, status = 'done', completed_at = $3 WHERE id = $1",
                )
                .bind(wait.id)
                .bind(json!(head_shas))
                .bind(now)
                .execute(tx.conn())
                .await?;
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
        processed += 1;
    }
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
    wait: &Wait,
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
