#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use async_graphql::{Request, Variables};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sanchoris_config::{Skill, builtin_skill, parse_config_yaml};
use sanchoris_core::db::{self, NewTask, TenantTx};
use sanchoris_core::model::{NamedDefinition, TaskDefinition};
use sanchoris_core::testing::TestPostgres;
use sanchoris_graphql::{
    AesGcmEncryptor, AuditExportPort, AuthUser, ChatDispatcher, CheckoutPort, Clock,
    ConfigLocation, ConfigProblem, ConfigRepositoryPort, ConfigSyncTrigger, GithubConnectUrl,
    GraphqlDeps, PlanChange, PortError, SanchorisSchema, SlackConnectUrl, TenantCheckout,
    TriggerProposal, build_schema,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
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
    pub admin: PgPool,
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

#[async_trait]
impl ChatDispatcher for Fakes {
    async fn dispatch(&self, tenant_id: Uuid) -> Result<(), PortError> {
        dispatch_chat(&self.admin, tenant_id)
            .await
            .map_err(|e| PortError::Failed(e.to_string()))
    }
}

async fn dispatch_chat(admin: &PgPool, tenant_id: Uuid) -> Result<(), db::DbError> {
    let events = sqlx::query(
        "SELECT id, key, event FROM sanchoris.inbound_events \
         WHERE tenant_id = $1 AND source = 'web' AND processed_at IS NULL ORDER BY received_at",
    )
    .bind(tenant_id)
    .fetch_all(admin)
    .await?;
    for row in events {
        let id: Uuid = row.get("id");
        let key: String = row.get("key");
        let event: Value = row.get("event");
        let detail = &event["detail"];
        let thread = detail["thread"].as_str().unwrap().to_string();
        let mut tx = TenantTx::begin(admin, tenant_id).await?;
        let trigger = sqlx::query(
            "SELECT id, config_revision_id FROM sanchoris.triggers \
             WHERE tenant_id = $1 AND state = 'ENABLED' AND event_pattern->'detail'->'scope' ? $2",
        )
        .bind(tenant_id)
        .bind(detail["scope"].as_str().unwrap())
        .fetch_optional(tx.conn())
        .await?;
        let existing = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM sanchoris.tasks WHERE channel->>'thread' = $1",
        )
        .bind(&thread)
        .fetch_one(tx.conn())
        .await?;
        if let (Some(trigger), 0) = (trigger, existing) {
            let trigger_id: Uuid = trigger.get("id");
            let recorded = db::record_trigger_event(
                &mut tx,
                &db::NewTriggerEvent {
                    trigger_id,
                    key: &key,
                    event: &event,
                },
            )
            .await?;
            db::set_trigger_event_outcome(
                &mut tx,
                recorded.id(),
                &db::TriggerEventOutcome::Started,
            )
            .await?;
            let definition = chat_definition();
            let member = detail["member_id"].as_str().map(|id| id.parse().unwrap());
            db::create_task(
                &mut tx,
                &NewTask {
                    trigger_id,
                    trigger_event_id: Some(recorded.id()),
                    config_revision_id: trigger.get("config_revision_id"),
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
        sqlx::query("UPDATE sanchoris.inbound_events SET processed_at = now() WHERE id = $1")
            .bind(id)
            .execute(tx.conn())
            .await?;
        tx.commit().await?;
    }
    Ok(())
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

pub struct World {
    pub pg: TestPostgres,
    pub admin: PgPool,
    pub app: PgPool,
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
    pub globex_task: Uuid,
    pub mika: Person,
    pub sota: Person,
    pub ken: Person,
    pub hana: Person,
    pub ren: Person,
    pub operator: Person,
}

async fn user(
    admin: &PgPool,
    name: &str,
    email: &str,
    github: Option<&str>,
    operator: bool,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO sanchoris.users (id, cognito_subject, email, display_name, github_user_id, github_login, is_operator) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(format!("google_{id}"))
    .bind(email)
    .bind(name)
    .bind(github.map(|_| i64::from(rand_id())))
    .bind(github)
    .bind(operator)
    .execute(admin)
    .await
    .unwrap();
    id
}

fn rand_id() -> i32 {
    i32::from_be_bytes(Uuid::new_v4().as_bytes()[..4].try_into().unwrap()).abs()
}

async fn member(admin: &PgPool, tenant: Uuid, user: Uuid, role: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO sanchoris.members (tenant_id, user_id, role) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(tenant)
    .bind(user)
    .bind(role)
    .fetch_one(admin)
    .await
    .unwrap()
}

impl World {
    pub async fn new() -> Self {
        let pg = TestPostgres::start().await;
        let admin = pg.admin_pool().await;
        let app = pg.app_pool().await;
        let now = Utc::now();
        let config = config();

        let mika_user = user(&admin, "Mika", "mika@acme.dev", Some("mika-dev"), false).await;
        let sota_user = user(&admin, "Sota", "sota@acme.dev", Some("sota-dev"), false).await;
        let ken_user = user(&admin, "Ken", "ken@acme.dev", None, false).await;
        let hana_user = user(&admin, "Hana", "hana@globex.dev", Some("hana-gx"), false).await;
        let ren_user = user(&admin, "Ren", "ren@acme.dev", None, false).await;
        let operator_user = user(&admin, "Operator", "ops@sancode.dev", None, true).await;

        let acme: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.tenants (slug, name, plan_id, period_start, tasks_this_period, workspace_seconds_this_period, status, created_by_user_id) \
             VALUES ('acme', 'Acme', 'pro', $1, 7, 7200, 'active', $2) RETURNING id",
        )
        .bind(now - Duration::days(3))
        .bind(mika_user)
        .fetch_one(&admin)
        .await
        .unwrap();
        let mika_member = member(&admin, acme, mika_user, "owner").await;
        let sota_member = member(&admin, acme, sota_user, "admin").await;
        let ken_member = member(&admin, acme, ken_user, "member").await;

        let globex: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.tenants (slug, name, plan_id, period_start, status, created_by_user_id) \
             VALUES ('globex', 'Globex', 'free', $1, 'active', $2) RETURNING id",
        )
        .bind(now - Duration::days(10))
        .bind(hana_user)
        .fetch_one(&admin)
        .await
        .unwrap();
        let hana_member = member(&admin, globex, hana_user, "owner").await;

        let config_id: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.configs (tenant_id, repository, root_path, branch, synced_at) \
             VALUES ($1, 'acme/sanchoris-config', '/', 'main', $2) RETURNING id",
        )
        .bind(acme)
        .bind(now - Duration::minutes(2))
        .fetch_one(&admin)
        .await
        .unwrap();
        let revision_id: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.config_revisions (tenant_id, config_id, commit_sha, parsed, validation_status) \
             VALUES ($1, $2, '9f2c4e1', '{}'::jsonb, 'valid') RETURNING id",
        )
        .bind(acme)
        .bind(config_id)
        .fetch_one(&admin)
        .await
        .unwrap();
        sqlx::query("UPDATE sanchoris.configs SET active_revision_id = $2 WHERE id = $1")
            .bind(config_id)
            .bind(revision_id)
            .execute(&admin)
            .await
            .unwrap();

        let project_id: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.projects (tenant_id, config_id, config_revision_id, name, system_prompt, source_path) \
             VALUES ($1, $2, $3, 'shop', 'You are working on the Acme shop.', 'sanchoris.yaml') RETURNING id",
        )
        .bind(acme)
        .bind(config_id)
        .bind(revision_id)
        .fetch_one(&admin)
        .await
        .unwrap();
        for (name, url, status) in [
            ("backend", "github.com/acme/shop-api", "failed"),
            ("frontend", "github.com/acme/shop-web", "succeeded"),
        ] {
            sqlx::query(
                "INSERT INTO sanchoris.repositories (tenant_id, project_id, name, url, prebuild_status, prebuild_at) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(acme)
            .bind(project_id)
            .bind(name)
            .bind(url)
            .bind(status)
            .bind(now - Duration::minutes(15))
            .execute(&admin)
            .await
            .unwrap();
        }
        for (name, session) in &config.sessions {
            sqlx::query(
                "INSERT INTO sanchoris.sessions (tenant_id, config_id, config_revision_id, name, definition) VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(acme)
            .bind(config_id)
            .bind(revision_id)
            .bind(name)
            .bind(serde_json::to_value(session).unwrap())
            .execute(&admin)
            .await
            .unwrap();
        }
        let skill = skill();
        sqlx::query(
            "INSERT INTO sanchoris.skills (tenant_id, config_id, config_revision_id, name, kind, description, skill_md, skill_yaml, definition, layout) \
             VALUES ($1, $2, $3, $4, 'reliable', $5, $6, $7, $8, $9)",
        )
        .bind(acme)
        .bind(config_id)
        .bind(revision_id)
        .bind(&skill.name)
        .bind(&skill.description)
        .bind(&skill.skill_md)
        .bind(&skill.skill_yaml)
        .bind(serde_json::to_value(skill.definition.as_ref().unwrap()).unwrap())
        .bind(serde_json::to_value(skill.definition.as_ref().unwrap().layout.as_ref()).unwrap())
        .execute(&admin)
        .await
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
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO sanchoris.triggers (tenant_id, config_id, config_revision_id, name, state, event_pattern, schedule_expression, start) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
            )
            .bind(acme)
            .bind(config_id)
            .bind(revision_id)
            .bind(name)
            .bind(serde_json::to_value(trigger.state).unwrap().as_str().unwrap().to_string())
            .bind(pattern)
            .bind(schedule)
            .bind(serde_json::to_value(&trigger.start).unwrap())
            .fetch_one(&admin)
            .await
            .unwrap();
            trigger_ids.insert(name.clone(), id);
        }
        let labeled_trigger = trigger_ids["labeled-issue"];
        let chat_trigger = trigger_ids["chat-all"];
        let nightly_trigger = trigger_ids["nightly"];
        sqlx::query(
            "INSERT INTO sanchoris.due_jobs (tenant_id, kind, ref_id, due_at) VALUES ($1, 'trigger_schedule', $2, $3)",
        )
        .bind(acme)
        .bind(nightly_trigger)
        .bind(now + Duration::hours(5))
        .execute(&admin)
        .await
        .unwrap();

        let org_credential: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.credentials (tenant_id, scope, name, provider, kind, account, status, subscription_plan, encrypted_value, kms_key_id, last_used_at) \
             VALUES ($1, 'org', 'acme-claude-team', 'claude', 'subscription', 'team@acme.dev', 'connected', 'Team', '\\x00', 'test-key', $2) RETURNING id",
        )
        .bind(acme)
        .bind(now - Duration::minutes(2))
        .fetch_one(&admin)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sanchoris.usage_snapshots (tenant_id, credential_id, captured_at, five_hour_utilization, five_hour_resets_at, seven_day_utilization, seven_day_resets_at) \
             VALUES ($1, $2, $3, 55, $4, 71, $5)",
        )
        .bind(acme)
        .bind(org_credential)
        .bind(now - Duration::minutes(1))
        .bind(now + Duration::hours(2))
        .bind(now + Duration::days(3))
        .execute(&admin)
        .await
        .unwrap();
        let mika_credential: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.credentials (tenant_id, scope, member_id, name, provider, kind, account, status, rejected_at, subscription_plan, encrypted_value, kms_key_id) \
             VALUES ($1, 'personal', $2, 'mika-codex', 'codex', 'subscription', 'mika@acme.dev', 'reconnect', $3, 'Pro', '\\x00', 'test-key') RETURNING id",
        )
        .bind(acme)
        .bind(mika_member)
        .bind(now - Duration::hours(1))
        .fetch_one(&admin)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sanchoris.credentials (tenant_id, scope, member_id, name, provider, kind, account, status, encrypted_value, kms_key_id) \
             VALUES ($1, 'personal', $2, 'ken-claude', 'claude', 'api_key', 'sk-ant-…9f2c', 'connected', '\\x00', 'test-key')",
        )
        .bind(acme)
        .bind(ken_member)
        .execute(&admin)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sanchoris.integrations (tenant_id, kind, status, github_installation_id, details) \
             VALUES ($1, 'github_installation', 'connected', 1001, $2)",
        )
        .bind(acme)
        .bind(json!({
            "account_login": "acme",
            "account_type": "Organization",
            "repositories": ["acme/shop-web", "acme/shop-api", "acme/sanchoris-config"],
        }))
        .execute(&admin)
        .await
        .unwrap();
        let invitation: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.invitations (tenant_id, email, role, status, invited_by_member_id, expires_at) \
             VALUES ($1, 'ren@acme.dev', 'member', 'pending', $2, $3) RETURNING id",
        )
        .bind(acme)
        .bind(mika_member)
        .bind(now + Duration::days(6))
        .fetch_one(&admin)
        .await
        .unwrap();

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
        let issue_event_id: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.trigger_events (tenant_id, trigger_id, key, event, received_at, result, evaluated_at) \
             VALUES ($1, $2, 'delivery-35', $3, $4, 'started', $4) RETURNING id",
        )
        .bind(acme)
        .bind(labeled_trigger)
        .bind(&issue_event)
        .bind(now - Duration::hours(2))
        .fetch_one(&admin)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sanchoris.trigger_events (tenant_id, trigger_id, key, event, received_at, result, reason, evaluated_at) \
             VALUES ($1, $2, 'delivery-36', $3, $4, 'skipped', 'the actor rin-dev (NONE) is not allowed by the trigger', $4)",
        )
        .bind(acme)
        .bind(labeled_trigger)
        .bind(json!({ "source": "github", "detail-type": "Issue Labeled", "detail": { "repository": "acme/shop-web", "sender": { "login": "rin-dev" } } }))
        .bind(now - Duration::hours(1))
        .execute(&admin)
        .await
        .unwrap();
        let issue_task: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.tasks (tenant_id, trigger_id, trigger_event_id, config_revision_id, name, title, status, nudges, definition, channel, requested_by_member_id, error, cause, created_at, started_at, updated_at) \
             VALUES ($1, $2, $3, $4, 'acme/shop-web#35', 'Fix flaky checkout test (acme/shop-web#35)', 'NEEDS_YOU', 1, $5, $6, $7, \
                     'Skill.FixesExhausted', 'The checks still fail after 3 fixes', $8, $8, $9) RETURNING id",
        )
        .bind(acme)
        .bind(labeled_trigger)
        .bind(issue_event_id)
        .bind(revision_id)
        .bind(serde_json::to_value(issue_definition()).unwrap())
        .bind(json!({ "source": "github", "repository": "acme/shop-web" }))
        .bind(mika_member)
        .bind(now - Duration::hours(2))
        .bind(now - Duration::minutes(5))
        .fetch_one(&admin)
        .await
        .unwrap();
        let issue_run: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.runs (tenant_id, task_id, session_name, worker, model, agent_session_id, status, turns, cost_usd, summary, started_at) \
             VALUES ($1, $2, 'shop', 'claude', 'claude-sonnet-5-5', 'sess-1', 'idle', 12, 3.5, 'Opened a PR', $3) RETURNING id",
        )
        .bind(acme)
        .bind(issue_task)
        .bind(now - Duration::hours(2))
        .fetch_one(&admin)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sanchoris.workspaces (tenant_id, run_id, compute, platform, repositories, branch, base_commit, started_at, stopped_at) \
             VALUES ($1, $2, 'standard', 'fargate', $3, $4, 'abc123', $5, $6)",
        )
        .bind(acme)
        .bind(issue_run)
        .bind(json!(["acme/shop-web", "acme/shop-api"]))
        .bind(format!("sanchoris/{issue_task}"))
        .bind(now - Duration::hours(2))
        .bind(now - Duration::minutes(30))
        .execute(&admin)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sanchoris.run_outbox (tenant_id, run_id, type, payload, dedupe_key, created_at) \
             VALUES ($1, $2, 'start', $3, 'start', $4), ($1, $2, 'user_message', $5, 'skill:1', $6)",
        )
        .bind(acme)
        .bind(issue_run)
        .bind(json!({ "instruction": "[sanchoris] Trigger labeled-issue started this task." }))
        .bind(now - Duration::hours(2) + Duration::seconds(1))
        .bind(json!({ "text": "[sanchoris] Skill open-pr-green-ci (run 1) needs you: CI failed." }))
        .bind(now - Duration::hours(1))
        .execute(&admin)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sanchoris.run_events (tenant_id, run_id, seq, type, payload, received_at, processed_at) \
             VALUES ($1, $2, 1, 'turn_end', $3, $4, $4)",
        )
        .bind(acme)
        .bind(issue_run)
        .bind(json!({ "turn": 3, "summary": "Opened a PR" }))
        .bind(now - Duration::minutes(90))
        .execute(&admin)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sanchoris.usage (tenant_id, run_id, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cost_usd, recorded_at) \
             VALUES ($1, $2, 1000, 200, 0, 0, 1.25, $3)",
        )
        .bind(acme)
        .bind(issue_run)
        .bind(now)
        .execute(&admin)
        .await
        .unwrap();
        let skill_run: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.skill_runs (tenant_id, task_id, run_id, number, skill_name, definition, input, status, current_state, state, error, cause, started_at, finished_at) \
             VALUES ($1, $2, $3, 1, 'open-pr-green-ci', $4, $5, 'FAILED', 'GiveUp', '{}'::jsonb, 'Skill.FixesExhausted', 'The checks still fail after 3 fixes', $6, $7) RETURNING id",
        )
        .bind(acme)
        .bind(issue_task)
        .bind(issue_run)
        .bind(serde_json::to_value(skill.definition.as_ref().unwrap()).unwrap())
        .bind(json!({ "Repos": ["acme/shop-web"], "Repo": "acme/shop-web", "Issue": 35 }))
        .bind(now - Duration::minutes(100))
        .bind(now - Duration::minutes(10))
        .fetch_one(&admin)
        .await
        .unwrap();
        for (sequence, state, state_type, status) in [
            (0, "Init", "Pass", "SUCCEEDED"),
            (1, "HandOff", "Task", "SUCCEEDED"),
            (2, "HandOff", "Task", "SUCCEEDED"),
            (3, "GiveUp", "Fail", "FAILED"),
        ] {
            sqlx::query(
                "INSERT INTO sanchoris.state_entries (tenant_id, skill_run_id, sequence, state_name, state_type, status, entered_at, exited_at, input, error) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $7, '{}'::jsonb, $8)",
            )
            .bind(acme)
            .bind(skill_run)
            .bind(sequence)
            .bind(state)
            .bind(state_type)
            .bind(status)
            .bind(now - Duration::minutes(60 - i64::from(sequence)))
            .bind((status == "FAILED").then_some("Skill.FixesExhausted"))
            .execute(&admin)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO sanchoris.outputs (tenant_id, skill_run_id, kind, repository, number, url, state, created_at) \
             VALUES ($1, $2, 'pull_request', 'acme/shop-web', 52, 'https://github.com/acme/shop-web/pull/52', 'open', $3)",
        )
        .bind(acme)
        .bind(skill_run)
        .bind(now)
        .execute(&admin)
        .await
        .unwrap();

        let chat_event = json!({
            "source": "web",
            "detail-type": "Message",
            "detail": { "scope": "all", "thread": "thread-1", "user": "Mika", "member_id": mika_member, "text": "What is failing?" },
        });
        sqlx::query(
            "INSERT INTO sanchoris.inbound_events (tenant_id, source, key, event, received_at, processed_at) \
             VALUES ($1, 'web', 'chat-1', $2, $3, $3)",
        )
        .bind(acme)
        .bind(&chat_event)
        .bind(now - Duration::minutes(20))
        .execute(&admin)
        .await
        .unwrap();
        let chat_event_id: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.trigger_events (tenant_id, trigger_id, key, event, received_at, result, evaluated_at) \
             VALUES ($1, $2, 'chat-1', $3, $4, 'started', $4) RETURNING id",
        )
        .bind(acme)
        .bind(chat_trigger)
        .bind(&chat_event)
        .bind(now - Duration::minutes(20))
        .fetch_one(&admin)
        .await
        .unwrap();
        let chat_task: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.tasks (tenant_id, trigger_id, trigger_event_id, config_revision_id, name, title, status, wait_reason, definition, channel, requested_by_member_id, created_at, started_at, updated_at) \
             VALUES ($1, $2, $3, $4, 'thread-1/1', 'What is failing?', 'WAITING', 'chat', $5, $6, $7, $8, $8, $8) RETURNING id",
        )
        .bind(acme)
        .bind(chat_trigger)
        .bind(chat_event_id)
        .bind(revision_id)
        .bind(serde_json::to_value(chat_definition()).unwrap())
        .bind(json!({ "source": "web", "thread": "thread-1", "channel": null }))
        .bind(mika_member)
        .bind(now - Duration::minutes(20))
        .fetch_one(&admin)
        .await
        .unwrap();
        let chat_run: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.runs (tenant_id, task_id, session_name, worker, model, status, started_at) \
             VALUES ($1, $2, 'all', 'claude', 'claude-sonnet-5-5', 'idle', $3) RETURNING id",
        )
        .bind(acme)
        .bind(chat_task)
        .bind(now - Duration::minutes(19))
        .fetch_one(&admin)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sanchoris.run_outbox (tenant_id, run_id, type, payload, dedupe_key, created_at) \
             VALUES ($1, $2, 'chat_post', $3, 'post-1', $4)",
        )
        .bind(acme)
        .bind(chat_run)
        .bind(json!({ "text": "The checkout spec is flaky.", "channel": { "source": "web", "thread": "thread-1" } }))
        .bind(now - Duration::minutes(15))
        .execute(&admin)
        .await
        .unwrap();

        let change_request: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.change_requests (tenant_id, config_id, pr_number, title, state, changed_definitions, validation_status, requested_by_task_id) \
             VALUES ($1, $2, 7, 'Tune labeled-issue', 'open', $3, 'valid', $4) RETURNING id",
        )
        .bind(acme)
        .bind(config_id)
        .bind(json!([{ "kind": "trigger", "name": "labeled-issue" }]))
        .bind(chat_task)
        .fetch_one(&admin)
        .await
        .unwrap();

        let globex_config: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.configs (tenant_id, repository, root_path, branch) VALUES ($1, 'globex/config', '/', 'main') RETURNING id",
        )
        .bind(globex)
        .fetch_one(&admin)
        .await
        .unwrap();
        let globex_revision: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.config_revisions (tenant_id, config_id, commit_sha, parsed, validation_status) \
             VALUES ($1, $2, 'beef', '{}'::jsonb, 'valid') RETURNING id",
        )
        .bind(globex)
        .bind(globex_config)
        .fetch_one(&admin)
        .await
        .unwrap();
        let globex_trigger: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.triggers (tenant_id, config_id, config_revision_id, name, state, event_pattern, start) \
             VALUES ($1, $2, $3, 'chat-all', 'ENABLED', $4, $5) RETURNING id",
        )
        .bind(globex)
        .bind(globex_config)
        .bind(globex_revision)
        .bind(serde_json::to_value(&chat_definition().trigger.definition["EventPattern"]).unwrap())
        .bind(serde_json::to_value(&chat_definition().trigger.definition["Start"]).unwrap())
        .fetch_one(&admin)
        .await
        .unwrap();
        let globex_task: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.tasks (tenant_id, trigger_id, config_revision_id, name, title, status, definition, requested_by_member_id) \
             VALUES ($1, $2, $3, 'secret', 'Globex secret task', 'NEEDS_YOU', $4, $5) RETURNING id",
        )
        .bind(globex)
        .bind(globex_trigger)
        .bind(globex_revision)
        .bind(serde_json::to_value(chat_definition()).unwrap())
        .bind(hana_member)
        .fetch_one(&admin)
        .await
        .unwrap();

        let recorder = Arc::new(Recorder::default());
        let fakes = Arc::new(Fakes {
            recorder: recorder.clone(),
            admin: admin.clone(),
        });
        let encryptor = Arc::new(AesGcmEncryptor::new(&[9; 32]));
        let schema = build_schema(
            app.clone(),
            GraphqlDeps {
                checkout: fakes.clone(),
                github: fakes.clone(),
                slack: fakes.clone(),
                config_sync: fakes.clone(),
                config_repository: fakes.clone(),
                chat: fakes.clone(),
                audit_export: fakes.clone(),
                encryptor: encryptor.clone(),
                clock: Arc::new(FixedClock(Mutex::new(now))),
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
            pg,
            admin,
            app,
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
            invitation,
            change_request,
            globex_task,
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
        let error = response
            .errors
            .first()
            .unwrap_or_else(|| panic!("{operation} succeeded: {:?}", response.data));
        error
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get("code"))
            .map(|code| code.to_string().trim_matches('"').to_string())
            .unwrap_or_else(|| format!("no code: {}", error.message))
    }

    pub async fn audit_actions(&self, tenant: Uuid) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT action FROM sanchoris.audit_events WHERE tenant_id = $1 ORDER BY at, action",
        )
        .bind(tenant)
        .fetch_all(&self.admin)
        .await
        .unwrap()
    }

    pub async fn audit(&self, tenant: Uuid, action: &str) -> Audit {
        let row = sqlx::query(
            "SELECT actor_kind, actor_ref, target_kind, target_ref, summary, metadata FROM sanchoris.audit_events \
             WHERE tenant_id = $1 AND action = $2",
        )
        .bind(tenant)
        .bind(action)
        .fetch_optional(&self.admin)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("no {action} audit event"));
        Audit {
            actor_kind: row.get("actor_kind"),
            actor_ref: row.get("actor_ref"),
            target_kind: row.get("target_kind"),
            target_ref: row.get("target_ref"),
            summary: row.get("summary"),
            metadata: row.get("metadata"),
        }
    }

    pub async fn scalar<T>(&self, sql: &'static str, id: Uuid) -> T
    where
        T: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + Send + Unpin,
    {
        sqlx::query_scalar::<_, T>(sql)
            .bind(id)
            .fetch_one(&self.admin)
            .await
            .unwrap()
    }
}

#[derive(Debug)]
pub struct Audit {
    pub actor_kind: String,
    pub actor_ref: String,
    pub target_kind: String,
    pub target_ref: String,
    pub summary: String,
    pub metadata: Value,
}
