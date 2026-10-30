use async_graphql::{Context, ID, Object, Result};
use chrono::{DateTime, Utc};
use sanchoris_config::{Checkout, REQUESTER};
use sanchoris_core::db::{self, REF_CONFIG_REVISION};
use sanchoris_core::items::{
    self, ConfigRevision as RevisionItem, Project as ProjectItem, Repository as RepositoryItem,
    SessionDef, SkillDef, Trigger as TriggerItem,
};
use sanchoris_core::keys;
use sanchoris_core::store::Listing;
use serde_json::Value;
use uuid::Uuid;

use crate::account::{Credential, Member};
use crate::context::{Scope, tx};
use crate::enums::{
    ChangeRequestState, ConfigObjectKind, ConfigStatus, Provider, UsageLimitResume,
    WorkspaceArchitecture, WorkspaceCompute,
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

pub(crate) async fn config_rows(ctx: &Context<'_>, scope: Scope) -> Result<Vec<items::Config>> {
    let mut rows = tx(ctx, scope.tenant_id)
        .query::<items::Config>(&keys::tenant_pk(scope.tenant_id), "CONFIG#")
        .await?;
    rows.sort_by(|a, b| (&a.repository, &a.root_path).cmp(&(&b.repository, &b.root_path)));
    Ok(rows)
}

pub(crate) async fn in_configs<E: sanchoris_core::item::Entity>(
    ctx: &Context<'_>,
    scope: Scope,
    prefix: fn(Uuid) -> String,
) -> Result<Vec<E>> {
    let mut found = Vec::new();
    let mut tx = tx(ctx, scope.tenant_id);
    for config in config_rows(ctx, scope).await? {
        if let Some(revision_id) = config.active_revision_id {
            found.extend(
                tx.query::<E>(
                    &keys::config_pk(scope.tenant_id, config.id),
                    &prefix(revision_id),
                )
                .await?,
            );
        }
    }
    Ok(found)
}

pub(crate) async fn active_revision(
    ctx: &Context<'_>,
    scope: Scope,
    config_id: Uuid,
) -> Result<Option<Uuid>> {
    Ok(tx(ctx, scope.tenant_id)
        .get::<items::Config>(&keys::config_key(scope.tenant_id, config_id))
        .await?
        .and_then(|config| config.active_revision_id))
}

pub(crate) async fn active_triggers(
    ctx: &Context<'_>,
    scope: Scope,
    config: &items::Config,
) -> Result<Vec<TriggerItem>> {
    Ok(tx(ctx, scope.tenant_id)
        .query::<TriggerItem>(&keys::config_pk(scope.tenant_id, config.id), "TRIGGER#")
        .await?
        .iter()
        .filter_map(|trigger| trigger.in_revision(config.active_revision_id))
        .collect())
}

pub(crate) struct Config {
    pub scope: Scope,
    pub row: items::Config,
}

impl Config {
    pub async fn all(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        Ok(config_rows(ctx, scope)
            .await?
            .into_iter()
            .map(|row| Config { scope, row })
            .collect())
    }

    pub async fn load(ctx: &Context<'_>, scope: Scope, config_id: Uuid) -> Result<Self> {
        tx(ctx, scope.tenant_id)
            .get::<items::Config>(&keys::config_key(scope.tenant_id, config_id))
            .await?
            .map(|row| Config { scope, row })
            .ok_or_else(|| not_found(format!("config {config_id}")))
    }

    async fn revision_rows(&self, ctx: &Context<'_>, limit: usize) -> Result<Vec<RevisionItem>> {
        Ok(tx(ctx, self.scope.tenant_id)
            .list::<RevisionItem>(
                &Listing::main(keys::config_pk(self.scope.tenant_id, self.row.id))
                    .prefix("REV#")
                    .descending()
                    .limit(limit),
                |_| true,
            )
            .await?)
    }

    fn pk(&self) -> String {
        keys::config_pk(self.scope.tenant_id, self.row.id)
    }
}

#[Object]
impl Config {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    /// The repository's current owner/name on GitHub, updated when the repository is renamed or transferred.
    async fn repository(&self) -> &str {
        &self.row.repository
    }

    /// The GitHub repository id that the config follows. Null when the config was disabled before its name resolved to a repository.
    async fn repository_id(&self) -> Option<ID> {
        self.row.repository_id.map(|id| ID(id.to_string()))
    }

    async fn status(&self) -> Result<ConfigStatus> {
        ConfigStatus::from_db(&self.row.status)
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

    /// The URL that creates a skill folder on GitHub.
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
            let mut tx = tx(ctx, self.scope.tenant_id);
            let location = db::locate(&mut tx, active, REF_CONFIG_REVISION)
                .await?
                .ok_or_else(|| not_found(format!("config revision {active}")))?;
            return Ok(ConfigRevision::Stored(Box::new(
                tx.require::<RevisionItem>(&location.target).await?,
            )));
        }
        Ok(match self.revision_rows(ctx, 1).await?.into_iter().next() {
            Some(row) => ConfigRevision::Stored(Box::new(row)),
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
            .revision_rows(ctx, usize::try_from(limit.unwrap_or(10))?)
            .await?
            .into_iter()
            .map(|row| ConfigRevision::Stored(Box::new(row)))
            .collect())
    }

    async fn synced_at(&self) -> Option<Timestamp> {
        ts_opt(self.row.synced_at)
    }

    /// Why the last sync failed, for example when GitHub is unreachable or a repository named in sanchoris.yaml is not in the GitHub App installation, or why the config is DISABLED. Null after a successful sync.
    async fn sync_error(&self) -> Option<&str> {
        self.row.sync_error.as_deref()
    }

    async fn trigger_count(&self, ctx: &Context<'_>) -> Result<i32> {
        let count = active_triggers(ctx, self.scope, &self.row).await?.len();
        Ok(i32::try_from(count)?)
    }

    async fn skill_count(&self, ctx: &Context<'_>) -> Result<i32> {
        let Some(revision_id) = self.row.active_revision_id else {
            return Ok(0);
        };
        let count = tx(ctx, self.scope.tenant_id)
            .query::<SkillDef>(&self.pk(), &keys::skills_prefix(revision_id))
            .await?
            .len();
        Ok(i32::try_from(count)?)
    }

    async fn projects(&self, ctx: &Context<'_>) -> Result<Vec<Project>> {
        Project::of_config(ctx, self.scope, self.row.id).await
    }

    async fn change_requests(
        &self,
        ctx: &Context<'_>,
        state: Option<ChangeRequestState>,
    ) -> Result<Vec<ChangeRequest>> {
        ChangeRequest::query(ctx, self.scope, Some(self.row.id), state).await
    }
}

pub(crate) enum ConfigRevision {
    Stored(Box<RevisionItem>),
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

    /// The revision number shown as r41.
    async fn number(&self) -> Result<i32> {
        match self {
            Self::Stored(row) => Ok(i32::try_from(row.number)?),
            Self::NotSynced { .. } => Ok(0),
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
    /// The file in the config repository, relative to the config root.
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

pub(crate) struct Project {
    scope: Scope,
    row: ProjectItem,
}

impl Project {
    fn sorted(scope: Scope, mut rows: Vec<ProjectItem>) -> Vec<Self> {
        rows.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));
        rows.into_iter().map(|row| Project { scope, row }).collect()
    }

    pub async fn all(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        Ok(Self::sorted(
            scope,
            in_configs::<ProjectItem>(ctx, scope, keys::projects_prefix).await?,
        ))
    }

    pub async fn of_config(ctx: &Context<'_>, scope: Scope, config_id: Uuid) -> Result<Vec<Self>> {
        let Some(revision_id) = active_revision(ctx, scope, config_id).await? else {
            return Ok(Vec::new());
        };
        let rows = tx(ctx, scope.tenant_id)
            .query::<ProjectItem>(
                &keys::config_pk(scope.tenant_id, config_id),
                &keys::projects_prefix(revision_id),
            )
            .await?;
        Ok(Self::sorted(scope, rows))
    }

    pub async fn by_name(
        ctx: &Context<'_>,
        scope: Scope,
        config_id: Uuid,
        name: &str,
    ) -> Result<Option<Self>> {
        let Some(revision_id) = active_revision(ctx, scope, config_id).await? else {
            return Ok(None);
        };
        Ok(tx(ctx, scope.tenant_id)
            .get::<ProjectItem>(&keys::project_key(
                scope.tenant_id,
                config_id,
                revision_id,
                name,
            ))
            .await?
            .map(|row| Project { scope, row }))
    }

    async fn repository_rows(&self, ctx: &Context<'_>) -> Result<Vec<RepositoryItem>> {
        let mut rows = tx(ctx, self.scope.tenant_id)
            .query::<RepositoryItem>(
                &keys::config_pk(self.scope.tenant_id, self.row.config_id),
                &keys::project_repositories_prefix(self.row.config_revision_id, &self.row.name),
            )
            .await?;
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(rows)
    }
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

    /// The latest warm build of its repositories.
    async fn warm_build(&self, ctx: &Context<'_>) -> Result<Option<WarmBuild>> {
        Ok(self
            .repository_rows(ctx)
            .await?
            .iter()
            .filter_map(warm_build)
            .max_by_key(|build| build.finished_at))
    }

    /// Sessions that clone this project.
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

fn warm_build(row: &RepositoryItem) -> Option<WarmBuild> {
    let finished_at = row.prebuild_at?;
    match row.prebuild_status.as_deref() {
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

pub(crate) struct Repository(RepositoryItem);

#[Object]
impl Repository {
    async fn id(&self) -> ID {
        ID(self.0.id.to_string())
    }

    /// The name inside the project, such as frontend.
    async fn name(&self) -> &str {
        &self.0.name
    }

    /// The repository's current owner/name on GitHub.
    async fn full_name(&self) -> &str {
        &self.0.full_name
    }

    async fn url(&self) -> String {
        format!("https://github.com/{}", self.0.full_name)
    }

    async fn warm_build(&self) -> Option<WarmBuild> {
        warm_build(&self.0)
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

impl Session {
    fn from_row(scope: Scope, row: SessionDef) -> Self {
        Session {
            scope,
            id: row.id.to_string(),
            config_id: row.config_id,
            name: row.name,
            definition: row.definition,
        }
    }

    pub async fn all(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        let mut rows = in_configs::<SessionDef>(ctx, scope, keys::sessions_prefix).await?;
        rows.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));
        Ok(rows
            .into_iter()
            .map(|row| Self::from_row(scope, row))
            .collect())
    }

    pub async fn by_name(
        ctx: &Context<'_>,
        scope: Scope,
        config_id: Uuid,
        name: &str,
    ) -> Result<Option<Self>> {
        let Some(revision_id) = active_revision(ctx, scope, config_id).await? else {
            return Ok(None);
        };
        Ok(tx(ctx, scope.tenant_id)
            .get::<SessionDef>(&keys::session_key(
                scope.tenant_id,
                config_id,
                revision_id,
                name,
            ))
            .await?
            .map(|row| Self::from_row(scope, row)))
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
                        .map(|row| row.full_name.clone())
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

    /// The credential name, or requester.
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

    /// Repositories cloned into the workspace, as owner/name.
    async fn repositories(&self, ctx: &Context<'_>) -> Result<Vec<String>> {
        self.repository_names(ctx).await
    }

    /// Repositories the GitHub token can reach, as owner/name or owner/*.
    async fn github_repositories(&self) -> Result<Vec<String>> {
        Ok(self.parsed()?.github.repos)
    }

    async fn compute(&self) -> Result<WorkspaceCompute> {
        WorkspaceCompute::from_db(self.parsed()?.workspace.compute.as_str())
    }

    async fn architecture(&self) -> Result<WorkspaceArchitecture> {
        WorkspaceArchitecture::from_db(self.parsed()?.workspace.architecture.as_str())
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

pub(crate) struct ChangeRequest {
    scope: Scope,
    config_repository: String,
    row: items::ChangeRequest,
}

impl ChangeRequest {
    pub fn new(scope: Scope, config_repository: String, row: items::ChangeRequest) -> Self {
        Self {
            scope,
            config_repository,
            row,
        }
    }

    pub async fn query(
        ctx: &Context<'_>,
        scope: Scope,
        config_id: Option<Uuid>,
        state: Option<ChangeRequestState>,
    ) -> Result<Vec<Self>> {
        let mut tx = tx(ctx, scope.tenant_id);
        let mut found = Vec::new();
        for config in config_rows(ctx, scope).await? {
            if config_id.is_some_and(|id| id != config.id) {
                continue;
            }
            let rows = tx
                .query::<items::ChangeRequest>(&keys::config_pk(scope.tenant_id, config.id), "CR#")
                .await?;
            found.extend(
                rows.into_iter()
                    .filter(|row| state.is_none_or(|state| row.state == state.as_db()))
                    .map(|row| ChangeRequest::new(scope, config.repository.clone(), row)),
            );
        }
        found.sort_by(|a, b| (b.row.created_at, a.row.id).cmp(&(a.row.created_at, b.row.id)));
        Ok(found)
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
}

#[Object]
impl ChangeRequest {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn config(&self, ctx: &Context<'_>) -> Result<Config> {
        Config::load(ctx, self.scope, self.row.config_id).await
    }

    async fn number(&self) -> Result<i32> {
        Ok(i32::try_from(self.row.pr_number)?)
    }

    async fn title(&self) -> &str {
        &self.row.title
    }

    async fn url(&self) -> String {
        format!(
            "https://github.com/{}/pull/{}",
            self.config_repository, self.row.pr_number
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

    /// The chat task that opened it. Null when a member opened it.
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
