#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use async_graphql::{Request, Variables};
use async_trait::async_trait;
use chrono::{DateTime, Duration, DurationRound, Utc};
use sanchoris_config::{Skill, builtin_skill, parse_config_yaml};
use sanchoris_core::db::{
    self, DbError, NewTask, REF_CONFIG_REVISION, REF_RUN, REF_SKILL_RUN, REF_TRIGGER,
    REF_TRIGGER_EVENT, Store, TenantTx, insert_ref,
};
use sanchoris_core::item::{Entity, Key, Table};
use sanchoris_core::items::{
    AuditEvent, ChangeRequest, ChatMessage, Config, ConfigRevision, Credential, Guard,
    InboundEvent, Integration, Invitation, InvitationEmail, Member, Membership, OutboxItem, Output,
    Pending, Project, Repository, Run, RunChannel, RunEventItem, SessionDef, SkillDef,
    SkillRunItem, StateEntryItem, Task, TaskName, Tenant, Trigger, TriggerEvent, UsageRecord,
    UsageSnapshot, User, Workspace, uniq,
};
use sanchoris_core::jobs::Relayed;
use sanchoris_core::keys;
use sanchoris_core::model::{
    NamedDefinition, SkillRunStatus, TaskDefinition, TaskStatus, TriggerEventResult, WaitReason,
};
use sanchoris_core::store::{Relay, relayed_from_item};
use sanchoris_core::testing::TestDynamo;
use sanchoris_graphql::{
    AesGcmEncryptor, AuditExportPort, AuthUser, ChatDispatcher, CheckoutPort, Clock,
    ConfigLocation, ConfigProblem, ConfigRepositoryPort, ConfigSyncTrigger, GithubConnectUrl,
    GraphqlDeps, PlanChange, PortError, SanchorisSchema, SlackConnectUrl, TenantCheckout,
    TriggerProposal, build_schema,
};
use serde_json::{Value, json};
use uuid::Uuid;

pub const CONFIG_YAML: &str = r#"
Version: 1
Projects:
  shop:
    SystemPrompt: You are working on the Acme shop.
    Repos:
      frontend:
        Repo: github.com/acme/shop-web
      backend:
        Repo: github.com/acme/shop-api
Sessions:
  shop:
    Worker: { Agent: claude, Model: claude-sonnet-5-5, Credential: acme-claude-team }
    Project: shop
    GitHub:
      Repos: [acme/shop-web, acme/shop-api]
      Permissions: { contents: write, pull_requests: write, issues: write, checks: read }
    Network: { Allow: [github.com] }
    UsageLimit: { Resume: auto }
    Workspace: { Compute: standard, IdleSeconds: 900 }
  all:
    Worker: { Agent: claude, Model: claude-sonnet-5-5, Credential: acme-claude-team }
    Repos: [acme/idea]
    GitHub:
      Repos: ["acme/*"]
      Permissions: { contents: read, issues: write }
    Network: { Allow: [github.com] }
    UsageLimit: { Resume: manual }
    Workspace: { Compute: standard, IdleSeconds: 900 }
Triggers:
  labeled-issue:
    State: ENABLED
    EventPattern:
      source: [github]
      detail-type: [Issue Labeled]
      detail:
        repository: [acme/shop-web, acme/shop-api]
        label: [sanchoris]
        sender: { login: [mika-dev, sota-dev] }
    Start:
      Session: shop
      Instruction: 'Resolve issue <number> in <repository>.'
      Skills: [open-pr-green-ci]
      RequiredSkills: [open-pr-green-ci]
      MaxNudges: 2
      Limits: { Turns: 60, CostUsd: 20, TimeoutSeconds: 14400, HeartbeatSeconds: 900, Children: { MaxDepth: 1, MaxCount: 5 } }
  chat-all:
    State: ENABLED
    EventPattern:
      source: [web]
      detail-type: [Message]
      detail:
        scope: [all]
    Start:
      Session: all
      Instruction: '<text>'
      Skills: []
      RequiredSkills: []
      MaxNudges: 0
      Limits: { Turns: 60, CostUsd: 10, TimeoutSeconds: 86400, HeartbeatSeconds: 900, Children: { MaxDepth: 1, MaxCount: 5 } }
  nightly:
    State: DISABLED
    ScheduleExpression: cron(0 3 * * ? *)
    Start:
      Session: shop
      Instruction: 'Run the nightly checks.'
      Skills: []
      RequiredSkills: []
      MaxNudges: 0
      Limits: { Turns: 60, CostUsd: 10, TimeoutSeconds: 86400, HeartbeatSeconds: 900, Children: { MaxDepth: 1, MaxCount: 5 } }
"#;

pub fn operations() -> String {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../apps/frontend/src/graphql"
    );
    let mut names = std::fs::read_dir(dir)
        .expect("the frontend operations exist")
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "graphql")
                && path
                    .file_name()
                    .is_some_and(|name| name != "schema.graphql")
        })
        .collect::<Vec<_>>();
    names.sort();
    names
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct FixedClock(pub Mutex<DateTime<Utc>>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

#[derive(Default)]
pub struct Recorder {
    pub syncs: Mutex<Vec<Uuid>>,
    pub proposals: Mutex<Vec<TriggerProposal>>,
    pub exports: Mutex<Vec<(String, Vec<u8>)>>,
    pub checks: Mutex<Vec<ConfigLocation>>,
    pub problems: Mutex<Vec<ConfigProblem>>,
    pub plan_change: Mutex<Option<PlanChange>>,
}

pub struct Fakes {
    pub recorder: Arc<Recorder>,
    pub store: Store,
    pub clock: Arc<FixedClock>,
}

#[async_trait]
impl CheckoutPort for Fakes {
    async fn tenant_checkout(&self, checkout: &TenantCheckout) -> Result<String, PortError> {
        Ok(format!(
            "https://checkout.stripe.test/{}/{}",
            checkout.plan_id, checkout.tenant_name
        ))
    }

    async fn change_plan(&self, _tenant_id: Uuid, plan_id: &str) -> Result<PlanChange, PortError> {
        Ok(self
            .recorder
            .plan_change
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(PlanChange::Checkout {
                url: format!("https://checkout.stripe.test/change/{plan_id}"),
            }))
    }

    async fn billing_portal(&self, tenant_id: Uuid) -> Result<String, PortError> {
        Ok(format!("https://billing.stripe.test/{tenant_id}"))
    }
}

impl GithubConnectUrl for Fakes {
    fn link_account(&self, user_id: Uuid) -> String {
        format!("https://sanchoris.test/auth/github?user={user_id}")
    }

    fn install_app(&self, tenant_id: Uuid, _user_id: Uuid) -> String {
        format!("https://github.com/apps/sanchoris/installations/new?state={tenant_id}")
    }
}

impl SlackConnectUrl for Fakes {
    fn install(&self, tenant_id: Uuid) -> Result<String, PortError> {
        Ok(format!("https://slack.test/oauth?state={tenant_id}"))
    }
}

#[async_trait]
impl ConfigSyncTrigger for Fakes {
    async fn request_sync(&self, _tenant_id: Uuid, config_id: Uuid) -> Result<(), PortError> {
        self.recorder.syncs.lock().unwrap().push(config_id);
        Ok(())
    }
}

#[async_trait]
impl ConfigRepositoryPort for Fakes {
    async fn check(
        &self,
        _tenant_id: Uuid,
        location: &ConfigLocation,
    ) -> Result<Vec<ConfigProblem>, PortError> {
        self.recorder.checks.lock().unwrap().push(location.clone());
        Ok(self.recorder.problems.lock().unwrap().clone())
    }

    async fn propose_trigger(
        &self,
        _tenant_id: Uuid,
        proposal: &TriggerProposal,
    ) -> Result<i32, PortError> {
        self.recorder
            .proposals
            .lock()
            .unwrap()
            .push(proposal.clone());
        Ok(42)
    }
}

#[async_trait]
impl AuditExportPort for Fakes {
    async fn publish(
        &self,
        _tenant_id: Uuid,
        file_name: &str,
        json_lines: Vec<u8>,
    ) -> Result<String, PortError> {
        self.recorder
            .exports
            .lock()
            .unwrap()
            .push((file_name.to_string(), json_lines));
        Ok(format!("https://exports.test/{file_name}"))
    }
}
pub fn config() -> sanchoris_config::Config {
    parse_config_yaml(CONFIG_YAML).expect("the test config parses")
}

pub fn skill() -> Skill {
    builtin_skill("open-pr-green-ci").unwrap()
}

fn trigger_definition(name: &str) -> Value {
    let config = config();
    let trigger = &config.triggers[name];
    let mut definition = serde_json::to_value(trigger).unwrap();
    definition
        .as_object_mut()
        .unwrap()
        .remove("InputTransformer");
    definition
}

pub fn issue_definition() -> TaskDefinition {
    let config = config();
    TaskDefinition {
        trigger: NamedDefinition {
            name: "labeled-issue".into(),
            definition: trigger_definition("labeled-issue"),
        },
        session: NamedDefinition {
            name: "shop".into(),
            definition: serde_json::to_value(&config.sessions["shop"]).unwrap(),
        },
        skills: vec![NamedDefinition {
            name: "open-pr-green-ci".into(),
            definition: serde_json::to_value(skill()).unwrap(),
        }],
    }
}

pub fn chat_definition() -> TaskDefinition {
    let config = config();
    TaskDefinition {
        trigger: NamedDefinition {
            name: "chat-all".into(),
            definition: trigger_definition("chat-all"),
        },
        session: NamedDefinition {
            name: "all".into(),
            definition: serde_json::to_value(&config.sessions["all"]).unwrap(),
        },
        skills: Vec::new(),
    }
}

#[derive(Debug, Clone)]
pub struct Person {
    pub user_id: Uuid,
    pub member_id: Option<Uuid>,
    pub email: String,
}

impl Person {
    pub fn auth(&self) -> AuthUser {
        AuthUser {
            user_id: self.user_id,
            cognito_subject: format!("google_{}", self.user_id),
            email: self.email.clone(),
        }
    }
}

#[async_trait]
impl ChatDispatcher for Fakes {
    async fn dispatch(&self, tenant_id: Uuid) -> Result<(), PortError> {
        dispatch_chat(&self.store, tenant_id, self.clock.now())
            .await
            .map_err(|e| PortError::Failed(e.to_string()))
    }
}

async fn dispatch_chat(store: &Store, tenant_id: Uuid, now: DateTime<Utc>) -> Result<(), DbError> {
    let mut tx = store.tx(tenant_id);
    let markers = tx
        .query::<Pending>(&keys::inbound_pk(tenant_id), "PEND#")
        .await?;
    for marker in markers {
        let mut inbound = tx.require::<InboundEvent>(&marker.target).await?;
        tx.delete(&marker.key)?;
        inbound.processed_at = Some(now);
        tx.put(&inbound)?;
        if inbound.source != "web" {
            continue;
        }
        let detail = inbound.event["detail"].clone();
        let thread = detail["thread"].as_str().unwrap().to_string();
        let scope = detail["scope"].as_str().unwrap().to_string();
        let mut found = None;
        for config in tx
            .query::<Config>(&keys::tenant_pk(tenant_id), "CONFIG#")
            .await?
        {
            for trigger in tx
                .query::<Trigger>(&keys::config_pk(tenant_id, config.id), "TRIGGER#")
                .await?
            {
                let accepts = trigger
                    .event_pattern
                    .as_ref()
                    .and_then(|pattern| pattern["detail"]["scope"].as_array().cloned())
                    .unwrap_or_default()
                    .iter()
                    .any(|candidate| candidate == scope.as_str());
                if trigger.is_active() && accepts {
                    found = Some(trigger);
                }
            }
        }
        let Some(trigger) = found else {
            continue;
        };
        let existing = tx
            .query::<TaskName>(
                &keys::trigger_pk(tenant_id, trigger.id),
                &format!("TASK#{thread}/"),
            )
            .await?;
        if !existing.is_empty() {
            continue;
        }
        let recorded = db::record_trigger_event(
            &mut tx,
            &db::NewTriggerEvent {
                trigger_id: trigger.id,
                key: &inbound.key,
                event: &inbound.event,
            },
        )
        .await?;
        db::set_trigger_event_outcome(&mut tx, recorded.id(), &db::TriggerEventOutcome::Started)
            .await?;
        let definition = chat_definition();
        let member = detail["member_id"].as_str().map(|id| id.parse().unwrap());
        db::create_task(
            &mut tx,
            &NewTask {
                trigger_id: trigger.id,
                trigger_event_id: Some(recorded.id()),
                config_revision_id: trigger.config_revision_id,
                name: &format!("{thread}/1"),
                title: detail["text"].as_str().unwrap(),
                definition: &definition,
                parent_task_id: None,
                channel: Some(&json!({ "source": "web", "thread": thread, "channel": null })),
                requested_by_member_id: member,
                blocked_by: &[],
            },
        )
        .await?;
    }
    tx.commit().await
}

pub struct World {
    pub dynamo: TestDynamo,
    pub store: Store,
    pub schema: SanchorisSchema,
    pub recorder: Arc<Recorder>,
    pub encryptor: Arc<AesGcmEncryptor>,
    pub now: DateTime<Utc>,
    pub acme: Uuid,
    pub globex: Uuid,
    pub config_id: Uuid,
    pub revision_id: Uuid,
    pub labeled_trigger: Uuid,
    pub chat_trigger: Uuid,
    pub nightly_trigger: Uuid,
    pub issue_task: Uuid,
    pub chat_task: Uuid,
    pub issue_run: Uuid,
    pub chat_run: Uuid,
    pub skill_run: Uuid,
    pub org_credential: Uuid,
    pub mika_credential: Uuid,
    pub invitation: Uuid,
    pub change_request: Uuid,
    pub globex_config: Uuid,
    pub globex_trigger: Uuid,
    pub globex_task: Uuid,
    pub globex_run: Uuid,
    pub globex_credential: Uuid,
    pub globex_invitation: Uuid,
    pub mika: Person,
    pub sota: Person,
    pub ken: Person,
    pub hana: Person,
    pub ren: Person,
    pub operator: Person,
}

fn rand_id() -> i64 {
    i64::from(u32::from_be_bytes(
        Uuid::new_v4().as_bytes()[..4].try_into().unwrap(),
    ))
}

async fn user(
    store: &Store,
    name: &str,
    email: &str,
    github: Option<&str>,
    operator: bool,
    now: DateTime<Utc>,
) -> Uuid {
    let id = Uuid::new_v4();
    let github_user_id = github.map(|_| rand_id());
    let mut tx = store.tx(Uuid::nil());
    tx.insert(&User {
        id,
        cognito_subject: format!("google_{id}"),
        email: email.into(),
        display_name: name.into(),
        github_user_id,
        github_login: github.map(str::to_string),
        is_operator: operator,
        created_at: now,
        updated_at: now,
    })
    .unwrap();
    tx.insert(&Guard::new(
        keys::uniq_key(uniq::COGNITO_SUBJECT, &format!("google_{id}")),
        id,
    ))
    .unwrap();
    tx.insert(&Guard::new(
        keys::uniq_key(uniq::EMAIL, &email.to_lowercase()),
        id,
    ))
    .unwrap();
    if let (Some(login), Some(github_id)) = (github, github_user_id) {
        tx.insert(&Guard::new(
            keys::uniq_key(uniq::GITHUB_USER, &github_id.to_string()),
            id,
        ))
        .unwrap();
        tx.insert(&Guard::new(
            keys::uniq_key(uniq::GITHUB_LOGIN, &login.to_lowercase()),
            id,
        ))
        .unwrap();
    }
    tx.commit().await.unwrap();
    id
}

#[allow(clippy::too_many_arguments)]
async fn tenant(
    store: &Store,
    slug: &str,
    name: &str,
    plan_id: &str,
    period_start: DateTime<Utc>,
    tasks_this_period: i64,
    workspace_seconds: i64,
    members: &[(Uuid, &str)],
    now: DateTime<Utc>,
) -> (Uuid, Vec<Uuid>) {
    let tenant_id = Uuid::new_v4();
    let mut tx = store.tx(tenant_id);
    tx.insert(&Tenant {
        id: tenant_id,
        slug: slug.into(),
        name: name.into(),
        plan_id: plan_id.into(),
        period_start,
        tasks_this_period,
        workspace_seconds_this_period: workspace_seconds,
        running_tasks: 0,
        status: "active".into(),
        suspended_reason: None,
        stripe_customer_id: None,
        created_by_user_id: members[0].0,
        created_at: now,
        updated_at: now,
    })
    .unwrap();
    tx.insert(&Guard::of_tenant(
        keys::uniq_key(uniq::SLUG, slug),
        tenant_id,
        tenant_id,
    ))
    .unwrap();
    let mut ids = Vec::new();
    for (user_id, role) in members {
        let member_id = Uuid::new_v4();
        tx.insert(&Member {
            id: member_id,
            tenant_id,
            user_id: *user_id,
            role: role.to_string(),
            joined_at: now,
            last_active_at: None,
        })
        .unwrap();
        tx.insert(&Membership {
            user_id: *user_id,
            tenant_id,
            member_id,
        })
        .unwrap();
        ids.push(member_id);
    }
    tx.commit().await.unwrap();
    (tenant_id, ids)
}

fn revision(tx: &mut TenantTx, config_id: Uuid, sha: &str, now: DateTime<Utc>) -> Uuid {
    let id = Uuid::new_v4();
    let item = ConfigRevision {
        id,
        tenant_id: tx.tenant_id(),
        config_id,
        number: 1,
        commit_sha: sha.into(),
        parsed: Some(json!({})),
        validation_status: "valid".into(),
        validation_messages: json!([]),
        created_at: now,
    };
    tx.insert(&item).unwrap();
    insert_ref(tx, id, REF_CONFIG_REVISION, item.key(), Some(config_id)).unwrap();
    id
}

fn config_item(
    tx: &mut TenantTx,
    repository: &str,
    revision_id: Option<Uuid>,
    config_id: Uuid,
    synced_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) {
    let tenant_id = tx.tenant_id();
    tx.insert(&Config {
        id: config_id,
        tenant_id,
        repository: repository.into(),
        root_path: "/".into(),
        branch: "main".into(),
        active_revision_id: revision_id,
        revisions: 1,
        synced_at,
        created_at: now,
        updated_at: now,
    })
    .unwrap();
    tx.insert(&Guard::new(
        keys::config_repository_key(tenant_id, repository, "/"),
        config_id,
    ))
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn trigger_item(
    tx: &mut TenantTx,
    config_id: Uuid,
    revision_id: Uuid,
    name: &str,
    state: &str,
    pattern: Option<Value>,
    schedule: Option<String>,
    start: Value,
    next_run_at: Option<DateTime<Utc>>,
) -> Uuid {
    let id = keys::trigger_id(config_id, name);
    let item = Trigger {
        id,
        tenant_id: tx.tenant_id(),
        config_id,
        config_revision_id: revision_id,
        name: name.into(),
        state: state.into(),
        event_pattern: pattern,
        schedule_expression: schedule,
        input_transformer: None,
        start,
        removed_at: None,
        next_run_at,
        schedule_message_id: next_run_at.map(|_| Uuid::new_v4()),
    };
    tx.insert(&item).unwrap();
    insert_ref(tx, id, REF_TRIGGER, item.key(), Some(config_id)).unwrap();
    id
}

#[allow(clippy::too_many_arguments)]
fn trigger_event(
    tx: &mut TenantTx,
    trigger_id: Uuid,
    key: &str,
    event: &Value,
    received_at: DateTime<Utc>,
    result: TriggerEventResult,
    reason: Option<&str>,
    task_id: Option<Uuid>,
) -> Uuid {
    let id = Uuid::new_v4();
    let item = TriggerEvent {
        id,
        tenant_id: tx.tenant_id(),
        trigger_id,
        key: key.into(),
        event: event.clone(),
        received_at,
        result: Some(result.as_str().into()),
        reason: reason.map(str::to_string),
        evaluated_at: Some(received_at),
        task_id,
    };
    tx.insert(&item).unwrap();
    insert_ref(tx, id, REF_TRIGGER_EVENT, item.key(), Some(trigger_id)).unwrap();
    id
}

pub struct NewTaskItem<'a> {
    pub id: Uuid,
    pub trigger_id: Uuid,
    pub trigger_event_id: Option<Uuid>,
    pub revision_id: Uuid,
    pub name: &'a str,
    pub title: &'a str,
    pub status: TaskStatus,
    pub definition: Value,
    pub channel: Option<Value>,
    pub requested_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn task_item(tx: &mut TenantTx, task: NewTaskItem<'_>) -> Task {
    let tenant_id = tx.tenant_id();
    let item = Task {
        id: task.id,
        tenant_id,
        trigger_id: task.trigger_id,
        trigger_event_id: task.trigger_event_id,
        config_revision_id: task.revision_id,
        name: task.name.into(),
        title: task.title.into(),
        status: task.status,
        wait_reason: None,
        resume_at: None,
        snoozed_until: None,
        nudges: 0,
        definition: task.definition,
        parent_task_id: None,
        channel: task.channel,
        start_payload: None,
        requested_by_member_id: task.requested_by,
        error: None,
        cause: None,
        runs: 0,
        skill_runs: 0,
        created_at: task.created_at,
        started_at: Some(task.created_at),
        finished_at: None,
        updated_at: task.updated_at,
    };
    tx.insert(&TaskName {
        tenant_id,
        trigger_id: task.trigger_id,
        name: task.name.into(),
        task_id: task.id,
        created_at: task.created_at,
    })
    .unwrap();
    item
}

fn run_item(
    tx: &mut TenantTx,
    task_id: Uuid,
    session: &str,
    started_at: DateTime<Utc>,
    change: impl FnOnce(&mut Run),
) -> Uuid {
    let tenant_id = tx.tenant_id();
    let id = Uuid::new_v4();
    let mut run = Run {
        id,
        tenant_id,
        task_id,
        number: 1,
        session_name: session.into(),
        worker: "claude".into(),
        model: "claude-sonnet-5-5".into(),
        agent_session_id: None,
        status: "idle".into(),
        turns: 0,
        cost_usd: 0.0,
        summary: None,
        error: None,
        resume_branch: None,
        record_s3_key: None,
        log_s3_key: None,
        last_heartbeat_at: None,
        started_at,
        finished_at: None,
        updated_at: started_at,
    };
    change(&mut run);
    tx.insert(&run).unwrap();
    tx.insert(&RunChannel {
        tenant_id,
        run_id: id,
        task_id,
        next_outbox_seq: SEEDED_OUTBOX + 1,
        instances: 0,
        connection_id: None,
        connection_instance: None,
        connection_started: false,
    })
    .unwrap();
    insert_ref(tx, id, REF_RUN, run.key(), Some(task_id)).unwrap();
    id
}

const SEEDED_OUTBOX: i64 = 2;

fn outbox(
    tx: &mut TenantTx,
    run_id: Uuid,
    seq: i64,
    kind: &str,
    payload: Value,
    created_at: DateTime<Utc>,
) {
    assert!(seq <= SEEDED_OUTBOX);
    tx.insert(&OutboxItem {
        id: Uuid::new_v4(),
        tenant_id: tx.tenant_id(),
        run_id,
        seq,
        kind: kind.into(),
        payload,
        dedupe_key: format!("{kind}-{seq}"),
        created_at,
        delivered_at: None,
    })
    .unwrap();
}

fn credential(
    tx: &mut TenantTx,
    name: &str,
    change: impl FnOnce(&mut Credential),
    now: DateTime<Utc>,
) -> Uuid {
    let id = Uuid::new_v4();
    let mut item = Credential {
        id,
        tenant_id: tx.tenant_id(),
        scope: "org".into(),
        member_id: None,
        name: name.into(),
        provider: "claude".into(),
        kind: "subscription".into(),
        account: String::new(),
        status: "connected".into(),
        subscription_plan: None,
        rejected_at: None,
        encrypted_value: "AA==".into(),
        kms_key_id: "test-key".into(),
        last_used_at: None,
        created_at: now,
        updated_at: now,
    };
    change(&mut item);
    tx.insert(&item).unwrap();
    tx.insert(&Guard::new(
        keys::credential_name_key(tx.tenant_id(), name),
        id,
    ))
    .unwrap();
    id
}

fn invitation(tx: &mut TenantTx, email: &str, invited_by: Uuid, now: DateTime<Utc>) -> Uuid {
    let tenant_id = tx.tenant_id();
    let id = Uuid::new_v4();
    tx.insert(&Invitation {
        id,
        tenant_id,
        email: email.into(),
        role: "member".into(),
        status: "pending".into(),
        invited_by_member_id: invited_by,
        expires_at: now + Duration::days(6),
        accepted_at: None,
        created_at: now,
    })
    .unwrap();
    tx.insert(&Guard::new(
        keys::pending_invitation_key(tenant_id, email),
        id,
    ))
    .unwrap();
    tx.insert(&InvitationEmail {
        email: email.into(),
        tenant_id,
        invitation_id: id,
    })
    .unwrap();
    id
}

impl World {
    pub async fn new() -> Self {
        let dynamo = TestDynamo::start().await;
        let store = dynamo.store(Relay::Stream);
        let now = Utc::now().duration_trunc(Duration::days(1)).unwrap() - Duration::hours(2);
        let config = config();
        let skill = skill();

        let mika_user = user(
            &store,
            "Mika",
            "mika@acme.dev",
            Some("mika-dev"),
            false,
            now,
        )
        .await;
        let sota_user = user(
            &store,
            "Sota",
            "sota@acme.dev",
            Some("sota-dev"),
            false,
            now,
        )
        .await;
        let ken_user = user(&store, "Ken", "ken@acme.dev", None, false, now).await;
        let hana_user = user(
            &store,
            "Hana",
            "hana@globex.dev",
            Some("hana-gx"),
            false,
            now,
        )
        .await;
        let ren_user = user(&store, "Ren", "ren@acme.dev", None, false, now).await;
        let operator_user = user(&store, "Operator", "ops@sancode.dev", None, true, now).await;

        let (acme, acme_members) = tenant(
            &store,
            "acme",
            "Acme",
            "pro",
            now - Duration::days(3),
            7,
            7200,
            &[
                (mika_user, "owner"),
                (sota_user, "admin"),
                (ken_user, "member"),
            ],
            now,
        )
        .await;
        let (mika_member, sota_member, ken_member) =
            (acme_members[0], acme_members[1], acme_members[2]);
        let (globex, globex_members) = tenant(
            &store,
            "globex",
            "Globex",
            "free",
            now - Duration::days(10),
            0,
            0,
            &[(hana_user, "owner")],
            now,
        )
        .await;
        let hana_member = globex_members[0];

        let mut tx = store.tx(acme);
        let config_id = Uuid::new_v4();
        let revision_id = revision(&mut tx, config_id, "9f2c4e1", now);
        config_item(
            &mut tx,
            "acme/sanchoris-config",
            Some(revision_id),
            config_id,
            Some(now - Duration::minutes(2)),
            now,
        );
        tx.insert(&Project {
            id: keys::config_entity_id(config_id, "project", "shop"),
            tenant_id: acme,
            config_id,
            config_revision_id: revision_id,
            name: "shop".into(),
            system_prompt: "You are working on the Acme shop.".into(),
            source_path: "sanchoris.yaml".into(),
        })
        .unwrap();
        for (name, url, status) in [
            ("backend", "github.com/acme/shop-api", "failed"),
            ("frontend", "github.com/acme/shop-web", "succeeded"),
        ] {
            tx.insert(&Repository {
                id: Uuid::new_v4(),
                tenant_id: acme,
                config_id,
                project: "shop".into(),
                name: name.into(),
                url: url.into(),
                warm_hook: None,
                start_hook: None,
                prebuild_status: Some(status.into()),
                prebuild_at: Some(now - Duration::minutes(15)),
            })
            .unwrap();
        }
        for (name, session) in &config.sessions {
            tx.insert(&SessionDef {
                id: keys::config_entity_id(config_id, "session", name),
                tenant_id: acme,
                config_id,
                config_revision_id: revision_id,
                name: name.clone(),
                definition: serde_json::to_value(session).unwrap(),
            })
            .unwrap();
        }
        tx.insert(&SkillDef {
            id: keys::config_entity_id(config_id, "skill", &skill.name),
            tenant_id: acme,
            config_id,
            config_revision_id: revision_id,
            name: skill.name.clone(),
            kind: "reliable".into(),
            description: skill.description.clone(),
            skill_md: skill.skill_md.clone(),
            skill_yaml: skill.skill_yaml.clone(),
            definition: Some(serde_json::to_value(skill.definition.as_ref().unwrap()).unwrap()),
            layout: Some(
                serde_json::to_value(skill.definition.as_ref().unwrap().layout.as_ref()).unwrap(),
            ),
        })
        .unwrap();
        let mut trigger_ids = std::collections::BTreeMap::new();
        for (name, trigger) in &config.triggers {
            let (pattern, schedule) = match &trigger.source {
                sanchoris_config::TriggerSource::EventPattern(pattern) => {
                    (Some(pattern.clone()), None)
                }
                sanchoris_config::TriggerSource::ScheduleExpression(expression) => {
                    (None, Some(expression.clone()))
                }
            };
            let next_run_at = schedule.as_ref().map(|_| now + Duration::hours(5));
            let id = trigger_item(
                &mut tx,
                config_id,
                revision_id,
                name,
                serde_json::to_value(trigger.state)
                    .unwrap()
                    .as_str()
                    .unwrap(),
                pattern.map(|pattern| serde_json::to_value(pattern).unwrap()),
                schedule,
                serde_json::to_value(&trigger.start).unwrap(),
                next_run_at,
            );
            trigger_ids.insert(name.clone(), id);
        }
        tx.commit().await.unwrap();
        let labeled_trigger = trigger_ids["labeled-issue"];
        let chat_trigger = trigger_ids["chat-all"];
        let nightly_trigger = trigger_ids["nightly"];

        let mut tx = store.tx(acme);
        let org_credential = credential(
            &mut tx,
            "acme-claude-team",
            |c| {
                c.account = "team@acme.dev".into();
                c.subscription_plan = Some("Team".into());
                c.last_used_at = Some(now - Duration::minutes(2));
            },
            now,
        );
        tx.insert(&UsageSnapshot {
            tenant_id: acme,
            credential_id: org_credential,
            captured_at: now - Duration::minutes(1),
            five_hour_utilization: 55.0,
            five_hour_resets_at: now + Duration::hours(2),
            seven_day_utilization: 71.0,
            seven_day_resets_at: now + Duration::days(3),
        })
        .unwrap();
        let mika_credential = credential(
            &mut tx,
            "mika-codex",
            |c| {
                c.scope = "personal".into();
                c.member_id = Some(mika_member);
                c.provider = "codex".into();
                c.account = "mika@acme.dev".into();
                c.status = "reconnect".into();
                c.rejected_at = Some(now - Duration::hours(1));
                c.subscription_plan = Some("Pro".into());
            },
            now,
        );
        credential(
            &mut tx,
            "ken-claude",
            |c| {
                c.scope = "personal".into();
                c.member_id = Some(ken_member);
                c.kind = "api_key".into();
                c.account = "sk-ant-…9f2c".into();
            },
            now,
        );
        let integration_id = Uuid::new_v4();
        tx.insert(&Integration {
            id: integration_id,
            tenant_id: acme,
            kind: "github_installation".into(),
            status: "connected".into(),
            github_installation_id: Some(1001),
            slack_team_id: None,
            details: json!({
                "account_login": "acme",
                "account_type": "Organization",
                "repositories": ["acme/shop-web", "acme/shop-api", "acme/sanchoris-config"],
            }),
            created_at: now,
            updated_at: now,
        })
        .unwrap();
        tx.insert(&Guard::of_tenant(
            keys::uniq_key(uniq::INSTALLATION, "1001"),
            integration_id,
            acme,
        ))
        .unwrap();
        let invitation_id = invitation(&mut tx, "ren@acme.dev", mika_member, now);
        tx.commit().await.unwrap();

        let mut tx = store.tx(acme);
        let issue_task = Uuid::new_v4();
        let issue_event = json!({
            "source": "github",
            "detail-type": "Issue Labeled",
            "detail": {
                "repository": "acme/shop-web",
                "installation": { "id": 1001 },
                "issue": { "number": 35, "title": "Fix flaky checkout test", "url": "https://github.com/acme/shop-web/issues/35" },
                "label": "sanchoris",
                "sender": { "login": "mika-dev" },
            },
        });
        let issue_event_id = trigger_event(
            &mut tx,
            labeled_trigger,
            "delivery-35",
            &issue_event,
            now - Duration::hours(2),
            TriggerEventResult::Started,
            None,
            Some(issue_task),
        );
        trigger_event(
            &mut tx,
            labeled_trigger,
            "delivery-36",
            &json!({ "source": "github", "detail-type": "Issue Labeled", "detail": { "repository": "acme/shop-web", "sender": { "login": "rin-dev" } } }),
            now - Duration::hours(1),
            TriggerEventResult::Skipped,
            Some("the actor rin-dev (NONE) is not allowed by the trigger"),
            None,
        );
        let mut task = task_item(
            &mut tx,
            NewTaskItem {
                id: issue_task,
                trigger_id: labeled_trigger,
                trigger_event_id: Some(issue_event_id),
                revision_id,
                name: "acme/shop-web#35",
                title: "Fix flaky checkout test (acme/shop-web#35)",
                status: TaskStatus::NeedsYou,
                definition: serde_json::to_value(issue_definition()).unwrap(),
                channel: Some(json!({ "source": "github", "repository": "acme/shop-web" })),
                requested_by: Some(mika_member),
                created_at: now - Duration::hours(2),
                updated_at: now - Duration::minutes(5),
            },
        );
        task.nudges = 1;
        task.error = Some("Skill.FixesExhausted".into());
        task.cause = Some("The checks still fail after 3 fixes".into());
        task.runs = 1;
        task.skill_runs = 1;
        tx.insert(&task).unwrap();
        let issue_run = run_item(
            &mut tx,
            issue_task,
            "shop",
            now - Duration::hours(2),
            |run| {
                run.agent_session_id = Some("sess-1".into());
                run.turns = 12;
                run.cost_usd = 3.5;
                run.summary = Some("Opened a PR".into());
            },
        );
        tx.insert(&Workspace {
            id: Uuid::new_v4(),
            tenant_id: acme,
            task_id: issue_task,
            run_id: issue_run,
            compute: "standard".into(),
            platform: "fargate".into(),
            platform_ref: None,
            repositories: vec!["acme/shop-web".into(), "acme/shop-api".into()],
            branch: format!("sanchoris/{issue_task}"),
            base_commit: Some("abc123".into()),
            started_at: now - Duration::hours(2),
            stopped_at: Some(now - Duration::minutes(30)),
        })
        .unwrap();
        outbox(
            &mut tx,
            issue_run,
            1,
            "start",
            json!({ "instruction": "[sanchoris] Trigger labeled-issue started this task." }),
            now - Duration::hours(2) + Duration::seconds(1),
        );
        outbox(
            &mut tx,
            issue_run,
            2,
            "user_message",
            json!({ "text": "[sanchoris] Skill open-pr-green-ci (run 1) needs you: CI failed." }),
            now - Duration::hours(1),
        );
        tx.insert(&RunEventItem {
            id: Uuid::new_v4(),
            tenant_id: acme,
            run_id: issue_run,
            seq: Some(1),
            kind: "turn_end".into(),
            payload: json!({ "turn": 3, "summary": "Opened a PR" }),
            received_at: now - Duration::minutes(90),
            processed_at: Some(now - Duration::minutes(90)),
            error: None,
        })
        .unwrap();
        tx.insert(&UsageRecord {
            id: Uuid::new_v4(),
            tenant_id: acme,
            task_id: issue_task,
            run_id: issue_run,
            input_tokens: 1000,
            output_tokens: 200,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: 1.25,
            recorded_at: now,
        })
        .unwrap();
        let skill_run = Uuid::new_v4();
        let skill_run_item = SkillRunItem {
            id: skill_run,
            tenant_id: acme,
            task_id: issue_task,
            run_id: Some(issue_run),
            number: 1,
            skill_name: "open-pr-green-ci".into(),
            definition: serde_json::to_value(skill.definition.as_ref().unwrap()).unwrap(),
            input: json!({ "Repos": ["acme/shop-web"], "Repo": "acme/shop-web", "Issue": 35 }),
            output: None,
            status: SkillRunStatus::Failed,
            current_state: Some("GiveUp".into()),
            state: json!({}),
            error: Some("Skill.FixesExhausted".into()),
            cause: Some("The checks still fail after 3 fixes".into()),
            started_at: now - Duration::minutes(100),
            finished_at: Some(now - Duration::minutes(10)),
            updated_at: now - Duration::minutes(10),
        };
        tx.insert(&skill_run_item).unwrap();
        insert_ref(
            &mut tx,
            skill_run,
            REF_SKILL_RUN,
            skill_run_item.key(),
            Some(issue_task),
        )
        .unwrap();
        for (sequence, state, state_type, status) in [
            (0, "Init", "Pass", "SUCCEEDED"),
            (1, "HandOff", "Task", "SUCCEEDED"),
            (2, "HandOff", "Task", "SUCCEEDED"),
            (3, "GiveUp", "Fail", "FAILED"),
        ] {
            let at = now - Duration::minutes(60 - sequence);
            tx.insert(&StateEntryItem {
                tenant_id: acme,
                task_id: issue_task,
                skill_run_id: skill_run,
                skill_run_number: 1,
                sequence,
                state_name: state.into(),
                state_type: state_type.into(),
                status: status.into(),
                entered_at: at,
                exited_at: Some(at),
                input: json!({}),
                output: None,
                error: (status == "FAILED").then(|| "Skill.FixesExhausted".into()),
                cause: None,
                attempts: 1,
                next_state: None,
            })
            .unwrap();
        }
        tx.insert(&Output {
            id: Uuid::new_v4(),
            tenant_id: acme,
            task_id: issue_task,
            skill_run_id: skill_run,
            skill_run_number: 1,
            kind: "pull_request".into(),
            repository: "acme/shop-web".into(),
            number: 52,
            url: "https://github.com/acme/shop-web/pull/52".into(),
            state: Some("open".into()),
            created_at: now,
            updated_at: now,
        })
        .unwrap();
        tx.commit().await.unwrap();

        let mut tx = store.tx(acme);
        let chat_task = Uuid::new_v4();
        let chat_event = json!({
            "source": "web",
            "detail-type": "Message",
            "detail": { "scope": "all", "thread": "thread-1", "user": "Mika", "member_id": mika_member, "text": "What is failing?" },
        });
        tx.insert(&InboundEvent {
            id: Uuid::new_v4(),
            tenant_id: acme,
            source: "web".into(),
            key: "chat-1".into(),
            event: chat_event.clone(),
            received_at: now - Duration::minutes(20),
            processed_at: Some(now - Duration::minutes(20)),
        })
        .unwrap();
        let chat_event_id = trigger_event(
            &mut tx,
            chat_trigger,
            "chat-1",
            &chat_event,
            now - Duration::minutes(20),
            TriggerEventResult::Started,
            None,
            Some(chat_task),
        );
        let mut task = task_item(
            &mut tx,
            NewTaskItem {
                id: chat_task,
                trigger_id: chat_trigger,
                trigger_event_id: Some(chat_event_id),
                revision_id,
                name: "thread-1/1",
                title: "What is failing?",
                status: TaskStatus::Waiting,
                definition: serde_json::to_value(chat_definition()).unwrap(),
                channel: Some(json!({ "source": "web", "thread": "thread-1", "channel": null })),
                requested_by: Some(mika_member),
                created_at: now - Duration::minutes(20),
                updated_at: now - Duration::minutes(20),
            },
        );
        task.wait_reason = Some(WaitReason::Chat);
        task.runs = 1;
        tx.insert(&task).unwrap();
        let chat_run = run_item(
            &mut tx,
            chat_task,
            "all",
            now - Duration::minutes(19),
            |_| {},
        );
        outbox(
            &mut tx,
            chat_run,
            1,
            "chat_post",
            json!({ "text": "The checkout spec is flaky.", "channel": { "source": "web", "thread": "thread-1" } }),
            now - Duration::minutes(15),
        );
        tx.insert(&ChatMessage {
            id: Uuid::new_v4(),
            tenant_id: acme,
            task_id: chat_task,
            run_id: Some(chat_run),
            outbox_id: None,
            channel: Some(json!({ "source": "web", "thread": "thread-1" })),
            author_kind: "agent".into(),
            text: "The checkout spec is flaky.".into(),
            created_at: now - Duration::minutes(15),
        })
        .unwrap();
        let change_request = Uuid::new_v4();
        tx.insert(&ChangeRequest {
            id: change_request,
            tenant_id: acme,
            config_id,
            pr_number: 7,
            title: "Tune labeled-issue".into(),
            state: "open".into(),
            changed_files: json!([]),
            changed_definitions: json!([{ "kind": "trigger", "name": "labeled-issue" }]),
            validation_status: Some("valid".into()),
            validation_messages: json!([]),
            requested_by_member_id: None,
            requested_by_task_id: Some(chat_task),
            created_at: now,
            updated_at: now,
        })
        .unwrap();
        tx.commit().await.unwrap();

        let mut tx = store.tx(globex);
        let globex_config = Uuid::new_v4();
        let globex_revision = revision(&mut tx, globex_config, "beef", now);
        config_item(
            &mut tx,
            "globex/config",
            Some(globex_revision),
            globex_config,
            None,
            now,
        );
        tx.insert(&SessionDef {
            id: keys::config_entity_id(globex_config, "session", "all"),
            tenant_id: globex,
            config_id: globex_config,
            config_revision_id: globex_revision,
            name: "all".into(),
            definition: serde_json::to_value(&config.sessions["all"]).unwrap(),
        })
        .unwrap();
        let globex_trigger = trigger_item(
            &mut tx,
            globex_config,
            globex_revision,
            "chat-all",
            "ENABLED",
            Some(chat_definition().trigger.definition["EventPattern"].clone()),
            None,
            chat_definition().trigger.definition["Start"].clone(),
            None,
        );
        let globex_task = Uuid::new_v4();
        let task = task_item(
            &mut tx,
            NewTaskItem {
                id: globex_task,
                trigger_id: globex_trigger,
                trigger_event_id: None,
                revision_id: globex_revision,
                name: "secret",
                title: "Globex secret task",
                status: TaskStatus::NeedsYou,
                definition: serde_json::to_value(chat_definition()).unwrap(),
                channel: Some(json!({ "source": "web", "thread": "secret", "channel": null })),
                requested_by: Some(hana_member),
                created_at: now,
                updated_at: now,
            },
        );
        tx.insert(&task).unwrap();
        let globex_run = run_item(&mut tx, globex_task, "all", now, |_| {});
        let globex_credential = credential(&mut tx, "globex-key", |_| {}, now);
        let globex_invitation = invitation(&mut tx, "zed@globex.dev", hana_member, now);
        tx.commit().await.unwrap();

        let recorder = Arc::new(Recorder::default());
        let clock = Arc::new(FixedClock(Mutex::new(now)));
        let fakes = Arc::new(Fakes {
            recorder: recorder.clone(),
            store: store.clone(),
            clock: clock.clone(),
        });
        let encryptor = Arc::new(AesGcmEncryptor::new(&[9; 32]));
        let schema = build_schema(
            store.clone(),
            GraphqlDeps {
                checkout: fakes.clone(),
                github: fakes.clone(),
                slack: fakes.clone(),
                config_sync: fakes.clone(),
                config_repository: fakes.clone(),
                chat: fakes.clone(),
                audit_export: fakes.clone(),
                encryptor: encryptor.clone(),
                clock,
            },
        );
        let person = |user_id, member_id, email: &str| Person {
            user_id,
            member_id,
            email: email.to_string(),
        };
        Self {
            mika: person(mika_user, Some(mika_member), "mika@acme.dev"),
            sota: person(sota_user, Some(sota_member), "sota@acme.dev"),
            ken: person(ken_user, Some(ken_member), "ken@acme.dev"),
            hana: person(hana_user, Some(hana_member), "hana@globex.dev"),
            ren: person(ren_user, None, "ren@acme.dev"),
            operator: person(operator_user, None, "ops@sancode.dev"),
            dynamo,
            store,
            schema,
            recorder,
            encryptor,
            now,
            acme,
            globex,
            config_id,
            revision_id,
            labeled_trigger,
            chat_trigger,
            nightly_trigger,
            issue_task,
            chat_task,
            issue_run,
            chat_run,
            skill_run,
            org_credential,
            mika_credential,
            invitation: invitation_id,
            change_request,
            globex_config,
            globex_trigger,
            globex_task,
            globex_run,
            globex_credential,
            globex_invitation,
        }
    }

    pub async fn run(
        &self,
        who: &Person,
        operation: &str,
        variables: Value,
    ) -> async_graphql::Response {
        self.schema
            .execute(
                Request::new(operations())
                    .operation_name(operation)
                    .variables(Variables::from_json(variables))
                    .data(who.auth()),
            )
            .await
    }

    pub async fn ok(&self, who: &Person, operation: &str, variables: Value) -> Value {
        let response = self.run(who, operation, variables).await;
        assert!(
            response.errors.is_empty(),
            "{operation} failed: {:?}",
            response.errors
        );
        response.data.into_json().unwrap()
    }

    pub async fn inline(
        &self,
        who: &Person,
        query: &str,
        variables: Value,
    ) -> async_graphql::Response {
        self.schema
            .execute(
                Request::new(query)
                    .variables(Variables::from_json(variables))
                    .data(who.auth()),
            )
            .await
    }

    pub async fn error_code(&self, who: &Person, operation: &str, variables: Value) -> String {
        let response = self.run(who, operation, variables).await;
        code_of(&response, operation)
    }

    pub async fn inline_code(&self, who: &Person, query: &str, variables: Value) -> String {
        let response = self.inline(who, query, variables).await;
        code_of(&response, query)
    }

    pub async fn get<E: Entity>(&self, key: &Key) -> Option<E> {
        self.store.get::<E>(key).await.unwrap()
    }

    pub async fn task(&self, tenant: Uuid, id: Uuid) -> Task {
        self.get::<Task>(&keys::task_key(tenant, id))
            .await
            .expect("the task exists")
    }

    pub async fn all<E: Entity>(&self) -> Vec<E> {
        self.dynamo.all::<E>().await
    }

    pub async fn audit_events(&self, tenant: Uuid) -> Vec<AuditEvent> {
        let mut events = self
            .dynamo
            .all::<AuditEvent>()
            .await
            .into_iter()
            .filter(|event| event.tenant_id == tenant)
            .collect::<Vec<_>>();
        events.sort_by(|a, b| (a.at, &a.action).cmp(&(b.at, &b.action)));
        events
    }

    pub async fn audit_actions(&self, tenant: Uuid) -> Vec<String> {
        self.audit_events(tenant)
            .await
            .into_iter()
            .map(|event| event.action)
            .collect()
    }

    pub async fn audit(&self, tenant: Uuid, action: &str) -> AuditEvent {
        self.audit_events(tenant)
            .await
            .into_iter()
            .find(|event| event.action == action)
            .unwrap_or_else(|| panic!("no {action} audit event"))
    }

    pub async fn messages(&self) -> Vec<Relayed> {
        self.dynamo
            .scan(Table::Main, "msg")
            .await
            .iter()
            .map(|item| {
                relayed_from_item(item)
                    .unwrap()
                    .expect("message items decode")
            })
            .collect()
    }
}

fn code_of(response: &async_graphql::Response, what: &str) -> String {
    let error = response
        .errors
        .first()
        .unwrap_or_else(|| panic!("{what} succeeded: {:?}", response.data));
    error
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get("code"))
        .map(|code| code.to_string().trim_matches('"').to_string())
        .unwrap_or_else(|| format!("no code: {}", error.message))
}
