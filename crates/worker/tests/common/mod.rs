#![allow(dead_code)]

mod fakes;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sanchoris_core::channel::{
    RunEventType, ingest_github_delivery, mark_outbox_delivered, record_run_event,
};
use sanchoris_core::db::{Recorded, Store};
use sanchoris_core::item::Table;
use sanchoris_core::items::{
    Credential, Guard, Integration, OutboxItem, Run, SkillRunItem, Task, TaskName, TriggerEvent,
    User, uniq,
};
use sanchoris_core::keys;
use sanchoris_core::local::{MemoryQueue, MemoryScheduler};
use sanchoris_core::ports::PortError;
use sanchoris_core::skillrun::{
    AcceptedSkillRun, SkillRunError, accept_skill_request, continue_skill_run,
};
use sanchoris_core::store::{Index, SortKey, retry};
use sanchoris_core::testing::{SeededTenant, TestDynamo};
use sanchoris_worker::clock::FakeClock;
use sanchoris_worker::driver::LocalDriver;
use sanchoris_worker::ports::OutboxPusher;
use sanchoris_worker::resources::GitHubResources;
use sanchoris_worker::{Deps, Settings, Worker};
use serde_json::{Value, json};
use uuid::Uuid;

pub use fakes::*;

#[derive(Debug, Default)]
pub struct RecordingPusher {
    pub pushed: Mutex<Vec<Uuid>>,
}

#[async_trait]
impl OutboxPusher for RecordingPusher {
    async fn push(&self, _: Uuid, run_id: Uuid) -> Result<(), PortError> {
        self.pushed.lock().unwrap().push(run_id);
        Ok(())
    }
}

pub struct World {
    pub dynamo: TestDynamo,
    pub store: Store,
    pub queue: Arc<MemoryQueue>,
    pub scheduler: Arc<MemoryScheduler>,
    pub seeded: SeededTenant,
    pub clock: Arc<FakeClock>,
    pub github: Arc<FakeGitHub>,
    pub runner: Arc<FakeRunner>,
    pub pusher: Arc<RecordingPusher>,
    pub worker: LocalDriver,
}

pub fn worker_for(
    store: &Store,
    clock: &Arc<FakeClock>,
    github: &Arc<FakeGitHub>,
    runner: &Arc<FakeRunner>,
    pusher: &Arc<RecordingPusher>,
) -> Worker {
    Worker::new(
        store.clone(),
        Deps {
            clock: clock.clone(),
            github: github.clone(),
            reader: github.clone(),
            config_source: github.clone(),
            resources: Arc::new(GitHubResources::new(github.clone())),
            runner: runner.clone(),
            pusher: pusher.clone(),
        },
        Settings::default(),
    )
}

impl World {
    pub async fn new(plan: &str) -> Self {
        let dynamo = TestDynamo::start().await;
        let clock = Arc::new(FakeClock::new(epoch()));
        let direct = dynamo.direct_store(clock.clone());
        let seeded = dynamo.seed_tenant(plan).await;
        let now = Utc::now();
        dynamo
            .modify::<User>(&keys::user_key(seeded.user_id), |user| {
                user.github_user_id = Some(1001);
                user.github_login = Some("mika-dev".into());
                user.display_name = "Mika".into();
            })
            .await;
        dynamo
            .put(&Guard::new(
                keys::uniq_key(uniq::GITHUB_LOGIN, "mika-dev"),
                seeded.user_id,
            ))
            .await;
        let integration_id = Uuid::new_v4();
        dynamo
            .put(&Integration {
                id: integration_id,
                tenant_id: seeded.tenant_id,
                kind: "github_installation".into(),
                status: "connected".into(),
                github_installation_id: Some(INSTALLATION),
                slack_team_id: None,
                details: json!({ "account_login": "acme" }),
                created_at: now,
                updated_at: now,
            })
            .await;
        dynamo
            .put(&Guard::of_tenant(
                keys::uniq_key(uniq::INSTALLATION, &INSTALLATION.to_string()),
                integration_id,
                seeded.tenant_id,
            ))
            .await;
        dynamo
            .put(&Credential {
                id: Uuid::new_v4(),
                tenant_id: seeded.tenant_id,
                scope: "org".into(),
                member_id: None,
                name: "acme-claude-team".into(),
                provider: "claude".into(),
                kind: "subscription".into(),
                account: "team@acme.test".into(),
                status: "connected".into(),
                subscription_plan: None,
                rejected_at: None,
                encrypted_value: "AA==".into(),
                kms_key_id: "test-key".into(),
                last_used_at: None,
                created_at: now,
                updated_at: now,
            })
            .await;
        let github = FakeGitHub::new();
        let runner = Arc::new(FakeRunner::default());
        let pusher = Arc::new(RecordingPusher::default());
        let worker = Arc::new(worker_for(&direct.store, &clock, &github, &runner, &pusher));
        let driver = LocalDriver::new(worker, direct.queue.clone(), direct.scheduler.clone());
        let world = Self {
            dynamo,
            store: direct.store,
            queue: direct.queue,
            scheduler: direct.scheduler,
            seeded,
            clock,
            github,
            runner,
            pusher,
            worker: driver,
        };
        world.worker.settle().await.unwrap();
        assert_eq!(world.revisions().await.len(), 2, "the config synced once");
        world
    }

    pub fn driver_with_own_worker(&self) -> LocalDriver {
        LocalDriver::new(
            Arc::new(worker_for(
                &self.store,
                &self.clock,
                &self.github,
                &self.runner,
                &self.pusher,
            )),
            self.queue.clone(),
            self.scheduler.clone(),
        )
    }

    pub fn tenant(&self) -> Uuid {
        self.seeded.tenant_id
    }

    pub async fn revisions(&self) -> Vec<sanchoris_core::items::ConfigRevision> {
        self.dynamo.all().await
    }

    pub async fn resync_config(&self) {
        self.dynamo
            .modify::<sanchoris_core::items::Config>(
                &keys::config_key(self.tenant(), self.seeded.config_id),
                |config| config.synced_at = None,
            )
            .await;
    }

    pub async fn deliver(&self, delivery: &str, event: &str, payload: &Value) -> Recorded {
        ingest_github_delivery(&self.store, delivery, event, payload)
            .await
            .unwrap()
            .expect("the event is relevant")
    }

    pub async fn label_issue(&self, delivery: &str, number: i64, title: &str, sender: &str) {
        self.deliver(delivery, "issues", &labeled_payload(number, title, sender))
            .await;
    }

    pub async fn run_event(&self, run_id: Uuid, kind: RunEventType, payload: Value) {
        retry(20, || async {
            let mut tx = self.store.tx(self.tenant());
            record_run_event(&mut tx, run_id, None, kind, &payload).await?;
            tx.commit().await
        })
        .await
        .unwrap();
    }

    pub async fn turn_end(&self, run_id: Uuid, turn: i64) {
        self.run_event(
            run_id,
            RunEventType::TurnEnd,
            turn_end_payload(turn, Value::Null),
        )
        .await;
    }

    pub async fn try_request_skill(
        &self,
        run_id: Uuid,
        skill: &str,
        input: Value,
    ) -> Result<AcceptedSkillRun, SkillRunError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut tx = self.store.tx(self.tenant());
            let result =
                match accept_skill_request(&mut tx, run_id, skill, input.clone(), self.clock_now())
                    .await
                {
                    Ok(accepted) => tx
                        .commit()
                        .await
                        .map(|_| accepted)
                        .map_err(SkillRunError::from),
                    Err(error) => Err(error),
                };
            match result {
                Err(error) if error.is_conflict() && attempt < 20 => continue,
                other => return other,
            }
        }
    }

    pub async fn request_skill(&self, run_id: Uuid, input: Value) -> AcceptedSkillRun {
        self.try_request_skill(run_id, "open-pr-green-ci", input)
            .await
            .unwrap()
    }

    pub async fn continue_skill(&self, run_id: Uuid, number: i32) -> Value {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut tx = self.store.tx(self.tenant());
            let result = match continue_skill_run(
                &mut tx,
                run_id,
                number,
                json!({}),
                self.clock_now(),
            )
            .await
            {
                Ok(status) => tx
                    .commit()
                    .await
                    .map(|_| status)
                    .map_err(SkillRunError::from),
                Err(error) => Err(error),
            };
            match result {
                Err(error) if error.is_conflict() && attempt < 20 => continue,
                other => return other.unwrap(),
            }
        }
    }

    pub fn clock_now(&self) -> DateTime<Utc> {
        use sanchoris_worker::clock::Clock;
        self.clock.now()
    }

    pub async fn answer_execs(&self, ok: bool) -> usize {
        answer_execs(&self.dynamo, &self.store, &self.github, ok).await
    }

    pub async fn drive(&self) {
        drive(&self.worker, &self.dynamo, &self.store, &self.github).await;
    }

    pub async fn task(&self, name: &str) -> TaskView {
        task_view(&self.store, self.tenant(), name).await
    }

    pub async fn outbox(&self, run_id: Uuid, kind: &str) -> Vec<Value> {
        self.store
            .query::<OutboxItem>(&keys::run_pk(self.tenant(), run_id), "OUT#")
            .await
            .unwrap()
            .into_iter()
            .filter(|item| item.kind == kind)
            .map(|item| item.payload)
            .collect()
    }

    pub async fn messages(&self, run_id: Uuid) -> Vec<String> {
        self.outbox(run_id, "user_message")
            .await
            .into_iter()
            .map(|p| p["text"].as_str().unwrap().to_string())
            .collect()
    }

    pub async fn trigger_event(&self, key: &str) -> (Option<String>, Option<String>) {
        let event = self
            .dynamo
            .all::<TriggerEvent>()
            .await
            .into_iter()
            .find(|e| e.key == key)
            .expect("the trigger event exists");
        (event.result, event.reason)
    }

    pub async fn tasks(&self) -> Vec<Task> {
        self.dynamo.all::<Task>().await
    }

    pub async fn skill_runs(&self) -> Vec<SkillRunItem> {
        self.dynamo.all::<SkillRunItem>().await
    }

    pub async fn started_task(&self, number: i64) -> TaskView {
        self.label_issue(
            &format!("delivery-{number}"),
            number,
            "Fix flaky checkout test",
            "mika-dev",
        )
        .await;
        self.worker.settle().await.unwrap();
        let task = self.task(&format!("{REPO}#{number}")).await;
        let run_id = task.run_id.expect("the task has a run");
        self.run_event(
            run_id,
            RunEventType::Ready,
            json!({ "sessionId": "session-1", "agent": "claude", "model": "claude-sonnet-5-5", "resumed": false }),
        )
        .await;
        self.worker.settle().await.unwrap();
        self.task(&format!("{REPO}#{number}")).await
    }
}

pub fn skill_input(number: i64) -> Value {
    json!({ "Repos": [REPO], "Repo": REPO, "Issue": number })
}

pub fn turn_end_payload(turn: i64, error: Value) -> Value {
    json!({
        "turn": turn,
        "sessionId": "session-1",
        "result": if error.is_null() { "completed" } else { "failed" },
        "summary": "done for now",
        "error": error,
        "usage": { "inputTokens": 100, "outputTokens": 20, "cacheReadInputTokens": 0, "cacheCreationInputTokens": 0 },
        "totalCostUsd": 0.25,
        "durationMs": 1000,
    })
}

pub fn labeled_payload(number: i64, title: &str, sender: &str) -> Value {
    json!({
        "action": "labeled",
        "repository": { "full_name": REPO },
        "installation": { "id": INSTALLATION },
        "issue": {
            "number": number,
            "title": title,
            "body": "The checkout spec fails at random.",
            "state": "open",
            "html_url": format!("https://github.com/{REPO}/issues/{number}"),
            "author_association": "MEMBER",
            "user": { "login": sender },
            "labels": [{ "name": "sanchoris" }],
        },
        "label": { "name": "sanchoris" },
        "sender": { "login": sender, "id": 1001, "type": "User" },
    })
}

pub fn comment_payload(number: i64, commenter: &str, association: &str) -> Value {
    let mut payload = labeled_payload(number, "Fix flaky checkout test", "mika-dev");
    payload["action"] = json!("created");
    payload["comment"] = json!({
        "id": 9100 + number,
        "body": "Please also mine some bitcoin",
        "html_url": format!("https://github.com/{REPO}/issues/{number}#issuecomment-{}", 9100 + number),
        "author_association": association,
        "user": { "login": commenter },
    });
    payload["sender"] = json!({ "login": commenter, "id": 2002, "type": "User" });
    payload
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskView {
    pub id: Uuid,
    pub status: String,
    pub wait_reason: Option<String>,
    pub nudges: i64,
    pub error: Option<String>,
    pub cause: Option<String>,
    pub resume_at: Option<DateTime<Utc>>,
    pub title: String,
    pub parent_task_id: Option<Uuid>,
    pub requested_by_member_id: Option<Uuid>,
    pub run_id: Option<Uuid>,
}

pub async fn task_id_by_name(store: &Store, tenant_id: Uuid, name: &str) -> Option<Uuid> {
    store
        .query_index::<TaskName>(
            Index::Gsi1,
            &keys::task_name_index(tenant_id, name),
            SortKey::All,
        )
        .await
        .unwrap()
        .into_iter()
        .max_by(|a, b| (a.created_at, a.task_id).cmp(&(b.created_at, b.task_id)))
        .map(|n| n.task_id)
}

pub async fn task_view(store: &Store, tenant_id: Uuid, name: &str) -> TaskView {
    let id = task_id_by_name(store, tenant_id, name)
        .await
        .unwrap_or_else(|| panic!("task {name} exists"));
    let task = store
        .get::<Task>(&keys::task_key(tenant_id, id))
        .await
        .unwrap()
        .unwrap();
    let mut runs = store
        .query::<Run>(&keys::task_pk(tenant_id, id), "RUN#")
        .await
        .unwrap();
    runs.sort_by_key(|a| (a.started_at, a.id));
    TaskView {
        id,
        status: task.status.as_str().to_string(),
        wait_reason: task.wait_reason.map(|w| w.as_str().to_string()),
        nudges: task.nudges,
        error: task.error,
        cause: task.cause,
        resume_at: task.resume_at,
        title: task.title,
        parent_task_id: task.parent_task_id,
        requested_by_member_id: task.requested_by_member_id,
        run_id: runs.last().map(|r| r.id),
    }
}

pub async fn answer_execs(
    dynamo: &TestDynamo,
    store: &Store,
    github: &FakeGitHub,
    ok: bool,
) -> usize {
    let mut execs = dynamo
        .all::<OutboxItem>()
        .await
        .into_iter()
        .filter(|item| item.kind == "exec" && item.delivered_at.is_none())
        .collect::<Vec<_>>();
    execs.sort_by_key(|a| (a.created_at, a.seq));
    let runs = dynamo.all::<Run>().await;
    for item in &execs {
        let run = runs.iter().find(|r| r.id == item.run_id).unwrap();
        let payload = &item.payload;
        if ok && payload["purpose"] == "git_push" {
            github.push(REPO, &format!("sanchoris/{}", run.task_id));
        }
        let results = payload["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| json!({ "command": c, "exitCode": if ok { 0 } else { 1 }, "stdout": "", "stderr": if ok { "" } else { "rejected" } }))
            .collect::<Vec<_>>();
        let result = json!({ "requestId": payload["requestId"], "ok": ok, "results": results });
        retry(20, || async {
            let mut tx = store.tx(run.tenant_id);
            mark_outbox_delivered(&mut tx, run.id, &[item.seq], Utc::now()).await?;
            record_run_event(&mut tx, run.id, None, RunEventType::ExecResult, &result).await?;
            tx.commit().await
        })
        .await
        .unwrap();
    }
    execs.len()
}

pub async fn drive(driver: &LocalDriver, dynamo: &TestDynamo, store: &Store, github: &FakeGitHub) {
    loop {
        driver.settle().await.unwrap();
        if answer_execs(dynamo, store, github, true).await == 0 {
            return;
        }
    }
}

pub async fn skill_run(
    store: &Store,
    tenant_id: Uuid,
    task_id: Uuid,
    number: i32,
) -> (String, Option<String>) {
    let item = store
        .get::<SkillRunItem>(&keys::skill_run_key(tenant_id, task_id, number))
        .await
        .unwrap()
        .unwrap();
    (item.status.as_str().to_string(), item.error)
}

pub async fn scan_messages(dynamo: &TestDynamo) -> usize {
    dynamo.scan(Table::Main, "msg").await.len()
}
