use async_graphql::{Context, ID, Object, Result};
use chrono::{DateTime, Utc};
use sanchoris_config::{Checkout, REQUESTER};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::account::{Credential, Member};
use crate::context::{Scope, tenant_tx};
use crate::enums::{
    ChangeRequestState, ConfigObjectKind, Provider, UsageLimitResume, WorkspaceCompute,
};
use crate::error::not_found;
use crate::scalars::{Timestamp, ts, ts_opt};
use crate::task::Task;

pub(crate) fn root_prefix(root: &str) -> String {
    let trimmed = root.trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}/")
    }
}

pub(crate) fn edit_url(repository: &str, branch: &str, root: &str, path: &str) -> String {
    format!(
        "https://github.com/{repository}/edit/{branch}/{}{path}",
        root_prefix(root)
    )
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ConfigRow {
    pub id: Uuid,
    pub repository: String,
    pub root_path: String,
    pub branch: String,
    pub active_revision_id: Option<Uuid>,
    pub synced_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

const CONFIG_COLUMNS: &str =
    "id, repository, root_path, branch, active_revision_id, synced_at, created_at";

pub(crate) struct Config {
    pub scope: Scope,
    pub row: ConfigRow,
}

impl Config {
    pub async fn all(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let rows = sqlx::query_as::<_, ConfigRow>(sql!(
            "SELECT {CONFIG_COLUMNS} FROM sanchoris.configs WHERE tenant_id = $1 ORDER BY repository, root_path"
        ))
        .bind(scope.tenant_id)
        .fetch_all(tx.conn())
        .await?;
        Ok(rows.into_iter().map(|row| Config { scope, row }).collect())
    }

    pub async fn load(ctx: &Context<'_>, scope: Scope, config_id: Uuid) -> Result<Self> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let row = sqlx::query_as::<_, ConfigRow>(sql!(
            "SELECT {CONFIG_COLUMNS} FROM sanchoris.configs WHERE id = $1"
        ))
        .bind(config_id)
        .fetch_optional(tx.conn())
        .await?
        .ok_or_else(|| not_found(format!("config {config_id}")))?;
        Ok(Config { scope, row })
    }

    async fn revision_rows(&self, ctx: &Context<'_>, limit: i64) -> Result<Vec<RevisionRow>> {
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        Ok(sqlx::query_as::<_, RevisionRow>(sql!(
            "{REVISION_SELECT} WHERE r.config_id = $1 ORDER BY r.created_at DESC, r.id DESC LIMIT $2"
        ))
        .bind(self.row.id)
        .bind(limit)
        .fetch_all(tx.conn())
        .await?)
    }
}

#[Object]
impl Config {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn repository(&self) -> &str {
        &self.row.repository
    }

    async fn root(&self) -> &str {
        &self.row.root_path
    }

    async fn branch(&self) -> &str {
        &self.row.branch
    }

    async fn url(&self) -> String {
        let prefix = root_prefix(&self.row.root_path);
        if prefix.is_empty() {
            format!("https://github.com/{}", self.row.repository)
        } else {
            format!(
                "https://github.com/{}/tree/{}/{}",
                self.row.repository,
                self.row.branch,
                prefix.trim_end_matches('/')
            )
        }
    }

    async fn new_skill_url(&self) -> String {
        format!(
            "https://github.com/{}/new/{}/{}skills",
            self.row.repository,
            self.row.branch,
            root_prefix(&self.row.root_path)
        )
    }

    async fn revision(&self, ctx: &Context<'_>) -> Result<ConfigRevision> {
        if let Some(active) = self.row.active_revision_id {
            let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
            let row = sqlx::query_as::<_, RevisionRow>(sql!("{REVISION_SELECT} WHERE r.id = $1"))
                .bind(active)
                .fetch_one(tx.conn())
                .await?;
            return Ok(ConfigRevision::Stored(row));
        }
        Ok(match self.revision_rows(ctx, 1).await?.into_iter().next() {
            Some(row) => ConfigRevision::Stored(row),
            None => ConfigRevision::NotSynced {
                config_id: self.row.id,
                created_at: self.row.created_at,
            },
        })
    }

    async fn revisions(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 10)] limit: Option<i32>,
    ) -> Result<Vec<ConfigRevision>> {
        Ok(self
            .revision_rows(ctx, i64::from(limit.unwrap_or(10)))
            .await?
            .into_iter()
            .map(ConfigRevision::Stored)
            .collect())
    }

    async fn synced_at(&self) -> Option<Timestamp> {
        ts_opt(self.row.synced_at)
    }

    async fn sync_error(&self) -> Option<String> {
        None
    }

    async fn trigger_count(&self, ctx: &Context<'_>) -> Result<i32> {
        count(
            ctx,
            self.scope,
            "SELECT count(*) FROM sanchoris.triggers WHERE config_id = $1 AND removed_at IS NULL",
            self.row.id,
        )
        .await
    }

    async fn skill_count(&self, ctx: &Context<'_>) -> Result<i32> {
        count(
            ctx,
            self.scope,
            "SELECT count(*) FROM sanchoris.skills WHERE config_id = $1",
            self.row.id,
        )
        .await
    }

    async fn projects(&self, ctx: &Context<'_>) -> Result<Vec<Project>> {
        Project::query(ctx, self.scope, Some(self.row.id)).await
    }

    async fn change_requests(
        &self,
        ctx: &Context<'_>,
        state: Option<ChangeRequestState>,
    ) -> Result<Vec<ChangeRequest>> {
        ChangeRequest::query(ctx, self.scope, Some(self.row.id), state).await
    }
}

pub(crate) async fn count(
    ctx: &Context<'_>,
    scope: Scope,
    sql: &'static str,
    id: Uuid,
) -> Result<i32> {
    let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
    let count = sqlx::query_scalar::<_, i64>(sql)
        .bind(id)
        .fetch_one(tx.conn())
        .await?;
    Ok(i32::try_from(count)?)
}

const REVISION_SELECT: &str = "SELECT r.id, r.commit_sha, r.validation_status, r.validation_messages, r.created_at, \
     (SELECT count(*) FROM sanchoris.config_revisions p WHERE p.config_id = r.config_id \
        AND (p.created_at, p.id) <= (r.created_at, r.id))::int AS number \
     FROM sanchoris.config_revisions r";

#[derive(Debug, Clone, FromRow)]
pub(crate) struct RevisionRow {
    id: Uuid,
    commit_sha: String,
    validation_status: String,
    validation_messages: Value,
    created_at: DateTime<Utc>,
    number: i32,
}

pub(crate) enum ConfigRevision {
    Stored(RevisionRow),
    NotSynced {
        config_id: Uuid,
        created_at: DateTime<Utc>,
    },
}

#[Object]
impl ConfigRevision {
    async fn id(&self) -> ID {
        match self {
            Self::Stored(row) => ID(row.id.to_string()),
            Self::NotSynced { config_id, .. } => ID(format!("{config_id}/r0")),
        }
    }

    async fn number(&self) -> i32 {
        match self {
            Self::Stored(row) => row.number,
            Self::NotSynced { .. } => 0,
        }
    }

    async fn commit_sha(&self) -> &str {
        match self {
            Self::Stored(row) => &row.commit_sha,
            Self::NotSynced { .. } => "",
        }
    }

    async fn committed_at(&self) -> Timestamp {
        match self {
            Self::Stored(row) => ts(row.created_at),
            Self::NotSynced { created_at, .. } => ts(*created_at),
        }
    }

    async fn validation(&self) -> Validation {
        match self {
            Self::Stored(row) => {
                Validation::from_stored(Some(&row.validation_status), &row.validation_messages)
            }
            Self::NotSynced { .. } => Validation {
                valid: true,
                errors: Vec::new(),
            },
        }
    }
}

pub(crate) struct Validation {
    valid: bool,
    errors: Vec<ValidationError>,
}

impl Validation {
    pub fn from_stored(status: Option<&str>, messages: &Value) -> Self {
        let errors = messages
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| ValidationError {
                        path: item["Location"].as_str().unwrap_or("").to_string(),
                        line: None,
                        column: None,
                        message: item["Message"].as_str().unwrap_or("").to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            valid: status != Some("invalid"),
            errors,
        }
    }
}

#[Object]
impl Validation {
    async fn valid(&self) -> bool {
        self.valid
    }

    async fn errors(&self) -> &[ValidationError] {
        &self.errors
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ValidationError {
    pub path: String,
    pub line: Option<i32>,
    pub column: Option<i32>,
    pub message: String,
}

#[Object]
impl ValidationError {
    async fn path(&self) -> &str {
        &self.path
    }

    async fn line(&self) -> Option<i32> {
        self.line
    }

    async fn column(&self) -> Option<i32> {
        self.column
    }

    async fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ProjectRow {
    id: Uuid,
    config_id: Uuid,
    name: String,
    system_prompt: String,
}

pub(crate) struct Project {
    scope: Scope,
    row: ProjectRow,
}

impl Project {
    pub async fn query(
        ctx: &Context<'_>,
        scope: Scope,
        config_id: Option<Uuid>,
    ) -> Result<Vec<Self>> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let rows = sqlx::query_as::<_, ProjectRow>(
            "SELECT id, config_id, name, system_prompt FROM sanchoris.projects \
             WHERE tenant_id = $1 AND ($2::uuid IS NULL OR config_id = $2) ORDER BY name, id",
        )
        .bind(scope.tenant_id)
        .bind(config_id)
        .fetch_all(tx.conn())
        .await?;
        Ok(rows.into_iter().map(|row| Project { scope, row }).collect())
    }

    pub async fn by_name(
        ctx: &Context<'_>,
        scope: Scope,
        config_id: Uuid,
        name: &str,
    ) -> Result<Option<Self>> {
        Ok(Self::query(ctx, scope, Some(config_id))
            .await?
            .into_iter()
            .find(|project| project.row.name == name))
    }

    async fn repository_rows(&self, ctx: &Context<'_>) -> Result<Vec<RepositoryRow>> {
        repository_rows(ctx, self.scope, self.row.id).await
    }
}

async fn repository_rows(
    ctx: &Context<'_>,
    scope: Scope,
    project_id: Uuid,
) -> Result<Vec<RepositoryRow>> {
    let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
    Ok(sqlx::query_as::<_, RepositoryRow>(
        "SELECT id, name, url, prebuild_status, prebuild_at FROM sanchoris.repositories \
         WHERE project_id = $1 ORDER BY name",
    )
    .bind(project_id)
    .fetch_all(tx.conn())
    .await?)
}

#[Object]
impl Project {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn name(&self) -> &str {
        &self.row.name
    }

    async fn config(&self, ctx: &Context<'_>) -> Result<Config> {
        Config::load(ctx, self.scope, self.row.config_id).await
    }

    async fn system_prompt(&self) -> Option<&str> {
        Some(self.row.system_prompt.as_str()).filter(|prompt| !prompt.is_empty())
    }

    async fn repositories(&self, ctx: &Context<'_>) -> Result<Vec<Repository>> {
        Ok(self
            .repository_rows(ctx)
            .await?
            .into_iter()
            .map(Repository)
            .collect())
    }

    async fn warm_build(&self, ctx: &Context<'_>) -> Result<Option<WarmBuild>> {
        Ok(self
            .repository_rows(ctx)
            .await?
            .iter()
            .filter_map(RepositoryRow::warm_build)
            .max_by_key(|build| build.finished_at))
    }

    async fn sessions(&self, ctx: &Context<'_>) -> Result<Vec<Session>> {
        Ok(Session::all(ctx, self.scope)
            .await?
            .into_iter()
            .filter(|session| {
                session.config_id == self.row.config_id
                    && session.definition.get("Project").and_then(Value::as_str)
                        == Some(self.row.name.as_str())
            })
            .collect())
    }
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct RepositoryRow {
    id: Uuid,
    name: String,
    url: String,
    prebuild_status: Option<String>,
    prebuild_at: Option<DateTime<Utc>>,
}

impl RepositoryRow {
    fn full_name(&self) -> &str {
        self.url.trim_start_matches("github.com/")
    }

    fn warm_build(&self) -> Option<WarmBuild> {
        let finished_at = self.prebuild_at?;
        match self.prebuild_status.as_deref() {
            Some("succeeded") => Some(WarmBuild {
                succeeded: true,
                finished_at,
            }),
            Some("failed") => Some(WarmBuild {
                succeeded: false,
                finished_at,
            }),
            _ => None,
        }
    }
}

pub(crate) struct Repository(RepositoryRow);

#[Object]
impl Repository {
    async fn id(&self) -> ID {
        ID(self.0.id.to_string())
    }

    async fn name(&self) -> &str {
        &self.0.name
    }

    async fn full_name(&self) -> &str {
        self.0.full_name()
    }

    async fn url(&self) -> String {
        format!("https://github.com/{}", self.0.full_name())
    }

    async fn warm_build(&self) -> Option<WarmBuild> {
        self.0.warm_build()
    }
}

pub(crate) struct WarmBuild {
    succeeded: bool,
    finished_at: DateTime<Utc>,
}

#[Object]
impl WarmBuild {
    async fn succeeded(&self) -> bool {
        self.succeeded
    }

    async fn finished_at(&self) -> Timestamp {
        ts(self.finished_at)
    }
}

pub(crate) struct Session {
    pub scope: Scope,
    pub id: String,
    pub config_id: Uuid,
    pub name: String,
    pub definition: Value,
}

#[derive(Debug, FromRow)]
struct SessionRow {
    id: Uuid,
    config_id: Uuid,
    name: String,
    definition: Value,
}

impl Session {
    pub async fn all(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT id, config_id, name, definition FROM sanchoris.sessions WHERE tenant_id = $1 ORDER BY name, id",
        )
        .bind(scope.tenant_id)
        .fetch_all(tx.conn())
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| Session {
                scope,
                id: row.id.to_string(),
                config_id: row.config_id,
                name: row.name,
                definition: row.definition,
            })
            .collect())
    }

    pub async fn by_name(
        ctx: &Context<'_>,
        scope: Scope,
        config_id: Uuid,
        name: &str,
    ) -> Result<Option<Self>> {
        Ok(Self::all(ctx, scope)
            .await?
            .into_iter()
            .find(|session| session.config_id == config_id && session.name == name))
    }

    pub fn parsed(&self) -> Result<sanchoris_config::Session> {
        serde_json::from_value(self.definition.clone()).map_err(|e| {
            async_graphql::Error::new(format!("session {} does not parse: {e}", self.name))
        })
    }

    pub async fn repository_names(&self, ctx: &Context<'_>) -> Result<Vec<String>> {
        match self.parsed()?.checkout {
            Checkout::Repos(repos) => Ok(repos),
            Checkout::Project(project) => {
                match Project::by_name(ctx, self.scope, self.config_id, &project).await? {
                    Some(project) => Ok(project
                        .repository_rows(ctx)
                        .await?
                        .iter()
                        .map(|row| row.full_name().to_string())
                        .collect()),
                    None => Ok(Vec::new()),
                }
            }
        }
    }
}

#[Object]
impl Session {
    async fn id(&self) -> ID {
        ID(self.id.clone())
    }

    async fn name(&self) -> &str {
        &self.name
    }

    async fn agent(&self) -> Result<Provider> {
        Provider::from_db(match self.parsed()?.worker.agent {
            sanchoris_config::Agent::Claude => "claude",
            sanchoris_config::Agent::Codex => "codex",
        })
    }

    async fn model(&self) -> Result<String> {
        Ok(self.parsed()?.worker.model)
    }

    async fn credential_name(&self) -> Result<String> {
        Ok(self.parsed()?.worker.credential)
    }

    async fn credential(&self, ctx: &Context<'_>) -> Result<Option<Credential>> {
        let name = self.parsed()?.worker.credential;
        if name == REQUESTER {
            return Ok(None);
        }
        Credential::by_name(ctx, self.scope, &name).await
    }

    async fn project(&self, ctx: &Context<'_>) -> Result<Option<Project>> {
        match self.parsed()?.checkout {
            Checkout::Project(name) => {
                Project::by_name(ctx, self.scope, self.config_id, &name).await
            }
            Checkout::Repos(_) => Ok(None),
        }
    }

    async fn repositories(&self, ctx: &Context<'_>) -> Result<Vec<String>> {
        self.repository_names(ctx).await
    }

    async fn github_repositories(&self) -> Result<Vec<String>> {
        Ok(self.parsed()?.github.repos)
    }

    async fn compute(&self) -> Result<WorkspaceCompute> {
        WorkspaceCompute::from_db(self.parsed()?.workspace.compute.as_str())
    }

    async fn idle_seconds(&self) -> Result<i32> {
        Ok(i32::try_from(self.parsed()?.workspace.idle_seconds)?)
    }

    async fn usage_limit_resume(&self) -> Result<UsageLimitResume> {
        Ok(match self.parsed()?.usage_limit.resume {
            sanchoris_config::ResumeMode::Auto => UsageLimitResume::Auto,
            sanchoris_config::ResumeMode::Manual => UsageLimitResume::Manual,
        })
    }
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ChangeRequestRow {
    id: Uuid,
    config_id: Uuid,
    config_repository: String,
    pr_number: i32,
    title: String,
    state: String,
    changed_definitions: Value,
    validation_status: Option<String>,
    validation_messages: Value,
    requested_by_member_id: Option<Uuid>,
    requested_by_task_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

pub(crate) struct ChangeRequest {
    scope: Scope,
    row: ChangeRequestRow,
}

impl ChangeRequest {
    pub async fn query(
        ctx: &Context<'_>,
        scope: Scope,
        config_id: Option<Uuid>,
        state: Option<ChangeRequestState>,
    ) -> Result<Vec<Self>> {
        Self::select(ctx, scope, config_id, state, None).await
    }

    pub async fn by_id(ctx: &Context<'_>, scope: Scope, id: Uuid) -> Result<Self> {
        Self::select(ctx, scope, None, None, Some(id))
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| not_found(format!("change request {id}")))
    }

    pub async fn open_for(
        ctx: &Context<'_>,
        scope: Scope,
        kind: ConfigObjectKind,
        name: &str,
    ) -> Result<Option<Self>> {
        Ok(
            Self::query(ctx, scope, None, Some(ChangeRequestState::Open))
                .await?
                .into_iter()
                .find(|change| {
                    change
                        .row
                        .changed_definitions
                        .as_array()
                        .is_some_and(|items| {
                            items
                                .iter()
                                .any(|item| item["kind"] == kind.as_db() && item["name"] == name)
                        })
                }),
        )
    }

    async fn select(
        ctx: &Context<'_>,
        scope: Scope,
        config_id: Option<Uuid>,
        state: Option<ChangeRequestState>,
        id: Option<Uuid>,
    ) -> Result<Vec<Self>> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let rows = sqlx::query_as::<_, ChangeRequestRow>(
            "SELECT cr.id, cr.config_id, c.repository AS config_repository, cr.pr_number, cr.title, cr.state, \
                    cr.changed_definitions, cr.validation_status, cr.validation_messages, \
                    cr.requested_by_member_id, cr.requested_by_task_id, cr.created_at \
             FROM sanchoris.change_requests cr JOIN sanchoris.configs c ON c.id = cr.config_id \
             WHERE cr.tenant_id = $1 AND ($2::uuid IS NULL OR cr.config_id = $2) \
               AND ($3::text IS NULL OR cr.state = $3) AND ($4::uuid IS NULL OR cr.id = $4) \
             ORDER BY cr.created_at DESC, cr.id",
        )
        .bind(scope.tenant_id)
        .bind(config_id)
        .bind(state.map(ChangeRequestState::as_db))
        .bind(id)
        .fetch_all(tx.conn())
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| ChangeRequest { scope, row })
            .collect())
    }
}

#[Object]
impl ChangeRequest {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn config(&self, ctx: &Context<'_>) -> Result<Config> {
        Config::load(ctx, self.scope, self.row.config_id).await
    }

    async fn number(&self) -> i32 {
        self.row.pr_number
    }

    async fn title(&self) -> &str {
        &self.row.title
    }

    async fn url(&self) -> String {
        format!(
            "https://github.com/{}/pull/{}",
            self.row.config_repository, self.row.pr_number
        )
    }

    async fn state(&self) -> Result<ChangeRequestState> {
        ChangeRequestState::from_db(&self.row.state)
    }

    async fn changes(&self) -> Result<Vec<ConfigObjectRef>> {
        self.row
            .changed_definitions
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|item| {
                Ok(ConfigObjectRef {
                    kind: ConfigObjectKind::from_db(item["kind"].as_str().unwrap_or(""))?,
                    name: item["name"].as_str().unwrap_or("").to_string(),
                })
            })
            .collect()
    }

    async fn validation(&self) -> Validation {
        Validation::from_stored(
            self.row.validation_status.as_deref(),
            &self.row.validation_messages,
        )
    }

    async fn opened_from_task(&self, ctx: &Context<'_>) -> Result<Option<Task>> {
        match self.row.requested_by_task_id {
            Some(task_id) => Task::find(ctx, self.scope, task_id).await,
            None => Ok(None),
        }
    }

    async fn opened_by(&self, ctx: &Context<'_>) -> Result<Option<Member>> {
        match self.row.requested_by_member_id {
            Some(member_id) => Member::find(ctx, self.scope, member_id).await,
            None => Ok(None),
        }
    }

    async fn opened_at(&self) -> Timestamp {
        ts(self.row.created_at)
    }
}

pub(crate) struct ConfigObjectRef {
    kind: ConfigObjectKind,
    name: String,
}

#[Object]
impl ConfigObjectRef {
    async fn kind(&self) -> ConfigObjectKind {
        self.kind
    }

    async fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_prefixes() {
        assert_eq!(root_prefix("/"), "");
        assert_eq!(root_prefix("tools/sanchoris/"), "tools/sanchoris/");
        assert_eq!(root_prefix("/tools/sanchoris"), "tools/sanchoris/");
        assert_eq!(
            edit_url(
                "acme/platform",
                "main",
                "tools/sanchoris/",
                "sanchoris.yaml"
            ),
            "https://github.com/acme/platform/edit/main/tools/sanchoris/sanchoris.yaml"
        );
    }
}
