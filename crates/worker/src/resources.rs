use std::sync::Arc;

use async_trait::async_trait;
use sanchoris_engine::{Invocation, ResourceOutcome};
use serde_json::{Value, json};

use crate::ports::{GitHubClient, GitHubError, InvocationContext, NewPullRequest, ResourceInvoker};

pub const ERROR_GITHUB: &str = "GitHub.RequestFailed";
pub const ERROR_INVALID_PARAMETERS: &str = "Resource.InvalidParameters";
pub const ERROR_UNSUPPORTED: &str = "Resource.Unsupported";

pub struct GitHubResources {
    client: Arc<dyn GitHubClient>,
}

#[derive(Debug)]
enum Failure {
    Parameters(String),
    GitHub(GitHubError),
}

impl From<GitHubError> for Failure {
    fn from(error: GitHubError) -> Self {
        Self::GitHub(error)
    }
}

impl Failure {
    fn outcome(self) -> ResourceOutcome {
        match self {
            Self::Parameters(cause) => ResourceOutcome::Failed {
                error: Some(ERROR_INVALID_PARAMETERS.to_string()),
                cause: Some(cause),
            },
            Self::GitHub(error) => ResourceOutcome::Failed {
                error: Some(ERROR_GITHUB.to_string()),
                cause: Some(error.to_string()),
            },
        }
    }
}

fn param<'a>(parameters: &'a Value, name: &str) -> Result<&'a Value, Failure> {
    parameters
        .get(name)
        .ok_or_else(|| Failure::Parameters(format!("{name} is required")))
}

fn param_str<'a>(parameters: &'a Value, name: &str) -> Result<&'a str, Failure> {
    param(parameters, name)?
        .as_str()
        .ok_or_else(|| Failure::Parameters(format!("{name} must be a string")))
}

fn param_i64(parameters: &Value, name: &str) -> Result<i64, Failure> {
    param(parameters, name)?
        .as_i64()
        .ok_or_else(|| Failure::Parameters(format!("{name} must be an integer")))
}

pub fn param_repos(parameters: &Value) -> Result<Vec<String>, String> {
    parameters
        .get("Repos")
        .and_then(Value::as_array)
        .ok_or_else(|| "Repos must be an array of owner/name".to_string())?
        .iter()
        .map(|repo| {
            repo.as_str()
                .filter(|r| r.contains('/'))
                .map(str::to_string)
                .ok_or_else(|| format!("{repo} is not owner/name"))
        })
        .collect()
}

pub fn comment_marker(context: &InvocationContext) -> String {
    format!(
        "<!-- sanchoris:{}:{} -->",
        context.skill_run_id, context.state_name
    )
}

impl GitHubResources {
    pub fn new(client: Arc<dyn GitHubClient>) -> Self {
        Self { client }
    }

    fn installation(&self, context: &InvocationContext, repo: &str) -> Result<i64, Failure> {
        context.installation_for(repo).map_err(Failure::Parameters)
    }

    async fn create_pull_requests(
        &self,
        context: &InvocationContext,
        parameters: &Value,
    ) -> Result<Value, Failure> {
        let repos = param_repos(parameters).map_err(Failure::Parameters)?;
        let title = match parameters.get("Title") {
            Some(Value::String(title)) => title.clone(),
            Some(_) => return Err(Failure::Parameters("Title must be a string".into())),
            None => context.task_title.clone(),
        };
        let body = match parameters.get("Body") {
            Some(Value::String(body)) => body.clone(),
            Some(_) => return Err(Failure::Parameters("Body must be a string".into())),
            None => format!("Opened by sanchoris for task {}.", context.task_name),
        };
        let mut opened = Vec::new();
        for repo in repos {
            let installation = self.installation(context, &repo)?;
            if let Some(existing) = self
                .client
                .find_open_pull_request(installation, &repo, &context.branch)
                .await?
            {
                opened.push(json!({ "Repo": repo, "Number": existing.number, "Url": existing.url, "Created": false }));
                continue;
            }
            let base = self.client.default_branch(installation, &repo).await?;
            match self
                .client
                .create_pull_request(
                    installation,
                    &repo,
                    &NewPullRequest {
                        head: &context.branch,
                        base: &base,
                        title: &title,
                        body: &body,
                    },
                )
                .await
            {
                Ok(created) => opened.push(
                    json!({ "Repo": repo, "Number": created.number, "Url": created.url, "Created": true }),
                ),
                Err(error) if error.status() == Some(422) && error.to_string().contains("No commits between") => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(Value::Array(opened))
    }

    async fn comment_on_issue(
        &self,
        context: &InvocationContext,
        invocation: &Invocation,
    ) -> Result<Value, Failure> {
        let parameters = &invocation.parameters;
        let repo = param_str(parameters, "Repo")?;
        let number = param_i64(parameters, "Number")?;
        let text = param_str(parameters, "Body")?;
        let marker = match param_str(parameters, "Mode")? {
            "update" => comment_marker(context),
            "create" => format!(
                "<!-- sanchoris:{}:{}:{} -->",
                context.skill_run_id, context.state_name, invocation.idempotency_key
            ),
            other => {
                return Err(Failure::Parameters(format!(
                    "Mode must be create or update, not {other:?}"
                )));
            }
        };
        let body = format!("{text}\n\n{marker}");
        let installation = self.installation(context, repo)?;
        let comments = self
            .client
            .list_issue_comments(installation, repo, number)
            .await?;
        let comment = match comments.iter().find(|c| c.body.contains(&marker)) {
            Some(existing) if existing.body == body => existing.clone(),
            Some(existing) => {
                self.client
                    .update_issue_comment(installation, repo, existing.id, &body)
                    .await?
            }
            None => {
                self.client
                    .create_issue_comment(installation, repo, number, &body)
                    .await?
            }
        };
        Ok(json!({ "Repo": repo, "Number": number, "CommentId": comment.id, "Url": comment.url }))
    }

    async fn merge_pull_request(
        &self,
        context: &InvocationContext,
        parameters: &Value,
    ) -> Result<Value, Failure> {
        let repo = param_str(parameters, "Repo")?;
        let number = param_i64(parameters, "Number")?;
        let installation = self.installation(context, repo)?;
        let current = self
            .client
            .get_pull_request(installation, repo, number)
            .await?;
        if current.merged {
            return Ok(
                json!({ "Repo": repo, "Number": number, "Merged": true, "Sha": Value::Null }),
            );
        }
        let merged = self
            .client
            .merge_pull_request(installation, repo, number)
            .await?;
        Ok(json!({ "Repo": repo, "Number": number, "Merged": merged.merged, "Sha": merged.sha }))
    }
}

#[async_trait]
impl ResourceInvoker for GitHubResources {
    async fn invoke(
        &self,
        context: &InvocationContext,
        invocation: &Invocation,
    ) -> ResourceOutcome {
        let result = match invocation.resource.name().as_str() {
            "github:createPullRequests" => {
                self.create_pull_requests(context, &invocation.parameters)
                    .await
            }
            "github:commentOnIssue" => self.comment_on_issue(context, invocation).await,
            "github:mergePullRequest" => {
                self.merge_pull_request(context, &invocation.parameters)
                    .await
            }
            other => {
                return ResourceOutcome::Failed {
                    error: Some(ERROR_UNSUPPORTED.to_string()),
                    cause: Some(format!("{other} is not an external resource")),
                };
            }
        };
        match result {
            Ok(output) => ResourceOutcome::Succeeded { output },
            Err(failure) => failure.outcome(),
        }
    }
}
