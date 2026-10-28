pub mod transport;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use sanchoris_config::ConfigFiles;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, OnceCell};

use crate::ports::{
    CheckRun, ConfigSource, GitHubClient, GitHubError, GitHubReader, IssueComment, IssueRef,
    IssueRelations, MergeResult, NewPullRequest, PullRequest,
};
use transport::{HttpRequest, HttpResponse, HttpTransport};

const PER_PAGE: usize = 100;

#[derive(Serialize)]
struct AppClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

pub struct AppAuth {
    app_id: String,
    key: EncodingKey,
    tokens: Mutex<HashMap<i64, (String, DateTime<Utc>)>>,
}

impl AppAuth {
    pub fn new(app_id: &str, private_key_pem: &str) -> Result<Self, GitHubError> {
        let key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
            .map_err(|error| GitHubError::Auth(format!("invalid private key: {error}")))?;
        Ok(Self {
            app_id: app_id.to_string(),
            key,
            tokens: Mutex::new(HashMap::new()),
        })
    }

    pub fn app_jwt(&self, now: DateTime<Utc>) -> Result<String, GitHubError> {
        let claims = AppClaims {
            iat: (now - Duration::seconds(60)).timestamp(),
            exp: (now + Duration::seconds(540)).timestamp(),
            iss: self.app_id.clone(),
        };
        jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &self.key)
            .map_err(|error| GitHubError::Auth(error.to_string()))
    }
}

pub struct GitHubApi {
    base_url: String,
    transport: Arc<dyn HttpTransport>,
    auth: AppAuth,
    app_login: OnceCell<String>,
}

const MAX_RETRIES: u32 = 2;
const MAX_RETRY_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

fn repeatable(request: &HttpRequest) -> bool {
    request.method == "GET"
        || request.url.ends_with("/graphql")
        || request.url.ends_with("/access_tokens")
}

fn retry_wait(
    request: &HttpRequest,
    response: &HttpResponse,
    attempt: u32,
    now: DateTime<Utc>,
) -> Option<std::time::Duration> {
    let rate_limited = response.status == 429
        || (response.status == 403
            && (response.header("x-ratelimit-remaining") == Some("0")
                || response.header("retry-after").is_some()
                || response
                    .body
                    .to_lowercase()
                    .contains("secondary rate limit")));
    let server_error = response.status >= 500 && repeatable(request);
    if !rate_limited && !server_error {
        return None;
    }
    if let Some(seconds) = response
        .header("retry-after")
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        return Some(std::time::Duration::from_secs(seconds));
    }
    if let Some(reset) = response
        .header("x-ratelimit-reset")
        .and_then(|value| value.trim().parse::<i64>().ok())
    {
        let seconds = u64::try_from(reset - now.timestamp()).unwrap_or(0);
        return Some(std::time::Duration::from_secs(seconds));
    }
    Some(std::time::Duration::from_secs(1 << attempt))
}

fn decode<T: serde::de::DeserializeOwned>(response: &HttpResponse) -> Result<T, GitHubError> {
    serde_json::from_str(&response.body)
        .map_err(|error| GitHubError::Decode(format!("{error}: {}", response.body)))
}

fn field<'a>(value: &'a Value, pointer: &str) -> Result<&'a Value, GitHubError> {
    value
        .pointer(pointer)
        .ok_or_else(|| GitHubError::Decode(format!("missing {pointer}")))
}

fn string(value: &Value, pointer: &str) -> Result<String, GitHubError> {
    field(value, pointer)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| GitHubError::Decode(format!("{pointer} is not a string")))
}

fn integer(value: &Value, pointer: &str) -> Result<i64, GitHubError> {
    field(value, pointer)?
        .as_i64()
        .ok_or_else(|| GitHubError::Decode(format!("{pointer} is not an integer")))
}

fn pull_request(value: &Value) -> Result<PullRequest, GitHubError> {
    Ok(PullRequest {
        number: integer(value, "/number")?,
        url: string(value, "/html_url")?,
        head_ref: string(value, "/head/ref")?,
        head_sha: string(value, "/head/sha")?,
        state: string(value, "/state")?,
        merged: value
            .get("merged")
            .and_then(Value::as_bool)
            .unwrap_or(value.get("merged_at").is_some_and(|m| !m.is_null())),
    })
}

fn comment(value: &Value) -> Result<IssueComment, GitHubError> {
    Ok(IssueComment {
        id: integer(value, "/id")?,
        body: string(value, "/body")?,
        url: string(value, "/html_url")?,
    })
}

fn issue_ref(value: &Value) -> Result<IssueRef, GitHubError> {
    Ok(IssueRef {
        repository: string(value, "/repository/nameWithOwner")?,
        number: integer(value, "/number")?,
    })
}

fn base64_text(path: &str, value: &Value) -> Result<String, GitHubError> {
    if value.get("encoding").and_then(Value::as_str) != Some("base64") {
        return Err(GitHubError::Decode(format!("{path} is not base64 encoded")));
    }
    let content: String = string(value, "/content")?
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content)
        .map_err(|error| GitHubError::Decode(format!("{path}: {error}")))?;
    String::from_utf8(bytes).map_err(|_| GitHubError::Decode(format!("{path} is not UTF-8")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationAccount {
    pub login: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoFile {
    pub sha: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange<'a> {
    pub path: &'a str,
    pub branch: &'a str,
    pub message: &'a str,
    pub text: &'a str,
    pub previous_sha: Option<&'a str>,
}

fn split_repo(repo: &str) -> Result<(&str, &str), GitHubError> {
    repo.split_once('/')
        .ok_or_else(|| GitHubError::Decode(format!("{repo:?} is not owner/name")))
}

impl GitHubApi {
    pub fn new(base_url: &str, transport: Arc<dyn HttpTransport>, auth: AppAuth) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            transport,
            auth,
            app_login: OnceCell::new(),
        }
    }

    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, GitHubError> {
        let mut attempt = 0;
        loop {
            let response = self.transport.send(request.clone()).await?;
            match retry_wait(&request, &response, attempt, Utc::now()) {
                Some(wait) if attempt < MAX_RETRIES && wait <= MAX_RETRY_WAIT => {
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                }
                _ => return Ok(response),
            }
        }
    }

    async fn app_slug(&self) -> Result<String, GitHubError> {
        let jwt = self.auth.app_jwt(Utc::now())?;
        let response = self
            .send(HttpRequest {
                method: "GET",
                url: format!("{}/app", self.base_url),
                headers: Self::headers(&jwt),
                body: None,
            })
            .await?;
        let body: Value = Self::check(response)?;
        string(&body, "/slug")
    }

    fn headers(token: &str) -> Vec<(String, String)> {
        vec![
            ("Authorization".into(), format!("Bearer {token}")),
            ("Accept".into(), "application/vnd.github+json".into()),
            ("X-GitHub-Api-Version".into(), "2022-11-28".into()),
            ("User-Agent".into(), "sanchoris".into()),
        ]
    }

    async fn installation_token(&self, installation: i64) -> Result<String, GitHubError> {
        let now = Utc::now();
        let mut tokens = self.auth.tokens.lock().await;
        if let Some((token, expires_at)) = tokens.get(&installation)
            && *expires_at - Duration::seconds(60) > now
        {
            return Ok(token.clone());
        }
        let jwt = self.auth.app_jwt(now)?;
        let response = self
            .send(HttpRequest {
                method: "POST",
                url: format!(
                    "{}/app/installations/{installation}/access_tokens",
                    self.base_url
                ),
                headers: Self::headers(&jwt),
                body: None,
            })
            .await?;
        let body: Value = Self::check(response)?;
        let token = string(&body, "/token")?;
        let expires_at = DateTime::parse_from_rfc3339(&string(&body, "/expires_at")?)
            .map_err(|error| GitHubError::Decode(error.to_string()))?
            .with_timezone(&Utc);
        tokens.insert(installation, (token.clone(), expires_at));
        Ok(token)
    }

    pub async fn scoped_installation_token(
        &self,
        installation: i64,
        repositories: Option<&[String]>,
        permissions: &std::collections::BTreeMap<String, String>,
    ) -> Result<(String, DateTime<Utc>), GitHubError> {
        let jwt = self.auth.app_jwt(Utc::now())?;
        let mut body = json!({ "permissions": permissions });
        if let Some(repositories) = repositories {
            body["repositories"] = json!(repositories);
        }
        let response = self
            .send(HttpRequest {
                method: "POST",
                url: format!(
                    "{}/app/installations/{installation}/access_tokens",
                    self.base_url
                ),
                headers: Self::headers(&jwt),
                body: Some(body),
            })
            .await?;
        let body: Value = Self::check(response)?;
        let token = string(&body, "/token")?;
        let expires_at = DateTime::parse_from_rfc3339(&string(&body, "/expires_at")?)
            .map_err(|error| GitHubError::Decode(error.to_string()))?
            .with_timezone(&Utc);
        Ok((token, expires_at))
    }

    fn check<T: serde::de::DeserializeOwned>(response: HttpResponse) -> Result<T, GitHubError> {
        if !(200..300).contains(&response.status) {
            return Err(GitHubError::Status {
                status: response.status,
                body: response.body,
            });
        }
        decode(&response)
    }

    async fn call(
        &self,
        installation: i64,
        method: &'static str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, GitHubError> {
        let token = self.installation_token(installation).await?;
        let response = self
            .send(HttpRequest {
                method,
                url: format!("{}{path}", self.base_url),
                headers: Self::headers(&token),
                body,
            })
            .await?;
        Self::check(response)
    }

    async fn paged(&self, installation: i64, path: &str) -> Result<Vec<Value>, GitHubError> {
        let separator = if path.contains('?') { '&' } else { '?' };
        let mut items = Vec::new();
        for page in 1.. {
            let value = self
                .call(
                    installation,
                    "GET",
                    &format!("{path}{separator}per_page={PER_PAGE}&page={page}"),
                    None,
                )
                .await?;
            let batch = value
                .as_array()
                .ok_or_else(|| GitHubError::Decode(format!("{path} did not return an array")))?
                .clone();
            let done = batch.len() < PER_PAGE;
            items.extend(batch);
            if done {
                break;
            }
        }
        Ok(items)
    }

    pub async fn app_installation(
        &self,
        installation: i64,
    ) -> Result<InstallationAccount, GitHubError> {
        let jwt = self.auth.app_jwt(Utc::now())?;
        let response = self
            .send(HttpRequest {
                method: "GET",
                url: format!("{}/app/installations/{installation}", self.base_url),
                headers: Self::headers(&jwt),
                body: None,
            })
            .await?;
        let body: Value = Self::check(response)?;
        Ok(InstallationAccount {
            login: string(&body, "/account/login")?,
            kind: string(&body, "/account/type")?,
        })
    }

    pub async fn installation_repositories(
        &self,
        installation: i64,
    ) -> Result<Vec<String>, GitHubError> {
        let mut names = Vec::new();
        for page in 1.. {
            let value = self
                .call(
                    installation,
                    "GET",
                    &format!("/installation/repositories?per_page={PER_PAGE}&page={page}"),
                    None,
                )
                .await?;
            let batch = field(&value, "/repositories")?
                .as_array()
                .ok_or_else(|| GitHubError::Decode("repositories is not an array".into()))?;
            for repository in batch {
                names.push(string(repository, "/full_name")?);
            }
            if batch.len() < PER_PAGE {
                break;
            }
        }
        Ok(names)
    }

    pub async fn file(
        &self,
        installation: i64,
        repo: &str,
        path: &str,
        reference: &str,
    ) -> Result<Option<RepoFile>, GitHubError> {
        let found = self
            .call(
                installation,
                "GET",
                &format!("/repos/{repo}/contents/{path}?ref={reference}"),
                None,
            )
            .await;
        match found {
            Ok(value) => Ok(Some(RepoFile {
                sha: string(&value, "/sha")?,
                text: base64_text(path, &value)?,
            })),
            Err(error) if error.status() == Some(404) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub async fn create_branch(
        &self,
        installation: i64,
        repo: &str,
        branch: &str,
        sha: &str,
    ) -> Result<(), GitHubError> {
        self.call(
            installation,
            "POST",
            &format!("/repos/{repo}/git/refs"),
            Some(json!({ "ref": format!("refs/heads/{branch}"), "sha": sha })),
        )
        .await?;
        Ok(())
    }

    pub async fn put_file(
        &self,
        installation: i64,
        repo: &str,
        change: &FileChange<'_>,
    ) -> Result<(), GitHubError> {
        let mut body = json!({
            "message": change.message,
            "content": base64::engine::general_purpose::STANDARD.encode(change.text),
            "branch": change.branch,
        });
        if let Some(sha) = change.previous_sha {
            body["sha"] = json!(sha);
        }
        self.call(
            installation,
            "PUT",
            &format!("/repos/{repo}/contents/{}", change.path),
            Some(body),
        )
        .await?;
        Ok(())
    }
}

#[async_trait]
impl GitHubClient for GitHubApi {
    async fn default_branch(&self, installation: i64, repo: &str) -> Result<String, GitHubError> {
        let value = self
            .call(installation, "GET", &format!("/repos/{repo}"), None)
            .await?;
        string(&value, "/default_branch")
    }

    async fn find_open_pull_request(
        &self,
        installation: i64,
        repo: &str,
        head_branch: &str,
    ) -> Result<Option<PullRequest>, GitHubError> {
        let (owner, _) = split_repo(repo)?;
        let value = self
            .call(
                installation,
                "GET",
                &format!("/repos/{repo}/pulls?state=open&head={owner}:{head_branch}"),
                None,
            )
            .await?;
        let items = value
            .as_array()
            .ok_or_else(|| GitHubError::Decode("pulls did not return an array".into()))?;
        items.first().map(pull_request).transpose()
    }

    async fn get_pull_request(
        &self,
        installation: i64,
        repo: &str,
        number: i64,
    ) -> Result<PullRequest, GitHubError> {
        let value = self
            .call(
                installation,
                "GET",
                &format!("/repos/{repo}/pulls/{number}"),
                None,
            )
            .await?;
        pull_request(&value)
    }

    async fn create_pull_request(
        &self,
        installation: i64,
        repo: &str,
        new: &NewPullRequest<'_>,
    ) -> Result<PullRequest, GitHubError> {
        let value = self
            .call(
                installation,
                "POST",
                &format!("/repos/{repo}/pulls"),
                Some(json!({ "title": new.title, "head": new.head, "base": new.base, "body": new.body })),
            )
            .await?;
        pull_request(&value)
    }

    async fn merge_pull_request(
        &self,
        installation: i64,
        repo: &str,
        number: i64,
    ) -> Result<MergeResult, GitHubError> {
        let value = self
            .call(
                installation,
                "PUT",
                &format!("/repos/{repo}/pulls/{number}/merge"),
                Some(json!({})),
            )
            .await?;
        Ok(MergeResult {
            merged: value
                .get("merged")
                .and_then(Value::as_bool)
                .ok_or_else(|| GitHubError::Decode("missing /merged".into()))?,
            sha: value.get("sha").and_then(Value::as_str).map(str::to_string),
        })
    }

    async fn list_issue_comments(
        &self,
        installation: i64,
        repo: &str,
        number: i64,
    ) -> Result<Vec<IssueComment>, GitHubError> {
        self.paged(
            installation,
            &format!("/repos/{repo}/issues/{number}/comments"),
        )
        .await?
        .iter()
        .map(comment)
        .collect()
    }

    async fn create_issue_comment(
        &self,
        installation: i64,
        repo: &str,
        number: i64,
        body: &str,
    ) -> Result<IssueComment, GitHubError> {
        let value = self
            .call(
                installation,
                "POST",
                &format!("/repos/{repo}/issues/{number}/comments"),
                Some(json!({ "body": body })),
            )
            .await?;
        comment(&value)
    }

    async fn update_issue_comment(
        &self,
        installation: i64,
        repo: &str,
        comment_id: i64,
        body: &str,
    ) -> Result<IssueComment, GitHubError> {
        let value = self
            .call(
                installation,
                "PATCH",
                &format!("/repos/{repo}/issues/comments/{comment_id}"),
                Some(json!({ "body": body })),
            )
            .await?;
        comment(&value)
    }

    async fn list_check_runs(
        &self,
        installation: i64,
        repo: &str,
        sha: &str,
    ) -> Result<Vec<CheckRun>, GitHubError> {
        let value = self
            .call(
                installation,
                "GET",
                &format!("/repos/{repo}/commits/{sha}/check-runs?per_page={PER_PAGE}"),
                None,
            )
            .await?;
        field(&value, "/check_runs")?
            .as_array()
            .ok_or_else(|| GitHubError::Decode("check_runs is not an array".into()))?
            .iter()
            .map(|run| {
                Ok(CheckRun {
                    name: string(run, "/name")?,
                    status: string(run, "/status")?,
                    conclusion: run
                        .get("conclusion")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    url: string(run, "/html_url")?,
                    summary: run
                        .pointer("/output/summary")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    text: run
                        .pointer("/output/text")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            })
            .collect()
    }
}

const RELATIONS_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { issue(number: $number) { parent { number repository { nameWithOwner } } blockedBy(first: 50) { nodes { number repository { nameWithOwner } } } } } }";

#[async_trait]
impl GitHubReader for GitHubApi {
    async fn issue_relations(
        &self,
        installation: i64,
        repo: &str,
        number: i64,
    ) -> Result<IssueRelations, GitHubError> {
        let (owner, name) = split_repo(repo)?;
        let value = self
            .call(
                installation,
                "POST",
                "/graphql",
                Some(json!({
                    "query": RELATIONS_QUERY,
                    "variables": { "owner": owner, "name": name, "number": number },
                })),
            )
            .await?;
        if let Some(errors) = value.get("errors").filter(|e| !e.is_null()) {
            return Err(GitHubError::Decode(format!("GraphQL errors: {errors}")));
        }
        let issue = field(&value, "/data/repository/issue")?;
        let parent = match issue.get("parent") {
            None | Some(Value::Null) => None,
            Some(parent) => Some(issue_ref(parent)?),
        };
        let blocked_by = field(issue, "/blockedBy/nodes")?
            .as_array()
            .ok_or_else(|| GitHubError::Decode("blockedBy.nodes is not an array".into()))?
            .iter()
            .map(issue_ref)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(IssueRelations { parent, blocked_by })
    }

    async fn app_login(&self) -> Result<String, GitHubError> {
        self.app_login
            .get_or_try_init(|| async { Ok(format!("{}[bot]", self.app_slug().await?)) })
            .await
            .cloned()
    }
}

fn config_path(root_path: &str, path: &str) -> Option<String> {
    let root = root_path.trim_matches('/');
    let relative = if root.is_empty() {
        path
    } else {
        path.strip_prefix(root)?.strip_prefix('/')?
    };
    let keep = relative == sanchoris_config::CONFIG_FILE
        || relative
            .strip_prefix("skills/")
            .and_then(|rest| rest.split_once('/'))
            .is_some_and(|(dir, file)| {
                !dir.is_empty()
                    && matches!(
                        file,
                        sanchoris_config::repository::SKILL_MD
                            | sanchoris_config::repository::SKILL_YAML
                    )
            });
    keep.then(|| relative.to_string())
}

#[async_trait]
impl ConfigSource for GitHubApi {
    async fn head_commit(
        &self,
        installation: i64,
        repo: &str,
        branch: &str,
    ) -> Result<String, GitHubError> {
        let value = self
            .call(
                installation,
                "GET",
                &format!("/repos/{repo}/commits/{branch}"),
                None,
            )
            .await?;
        string(&value, "/sha")
    }

    async fn fetch(
        &self,
        installation: i64,
        repo: &str,
        root_path: &str,
        sha: &str,
    ) -> Result<ConfigFiles, GitHubError> {
        let tree = self
            .call(
                installation,
                "GET",
                &format!("/repos/{repo}/git/trees/{sha}?recursive=1"),
                None,
            )
            .await?;
        if tree.get("truncated").and_then(Value::as_bool) == Some(true) {
            return Err(GitHubError::Decode(format!(
                "the tree of {repo}@{sha} is too large to read in one request"
            )));
        }
        let mut files = ConfigFiles::new();
        for entry in field(&tree, "/tree")?
            .as_array()
            .ok_or_else(|| GitHubError::Decode("tree is not an array".into()))?
        {
            if entry.get("type").and_then(Value::as_str) != Some("blob") {
                continue;
            }
            let Some(path) = config_path(root_path, &string(entry, "/path")?) else {
                continue;
            };
            let blob = self
                .call(
                    installation,
                    "GET",
                    &format!("/repos/{repo}/git/blobs/{}", string(entry, "/sha")?),
                    None,
                )
                .await?;
            let text = base64_text(&path, &blob)?;
            files.insert(path, text);
        }
        Ok(files)
    }

    async fn commit_author(
        &self,
        installation: i64,
        repo: &str,
        sha: &str,
    ) -> Result<String, GitHubError> {
        let value = self
            .call(
                installation,
                "GET",
                &format!("/repos/{repo}/commits/{sha}"),
                None,
            )
            .await?;
        match value.pointer("/author/login").and_then(Value::as_str) {
            Some(login) => Ok(login.to_string()),
            None => {
                let name = string(&value, "/commit/author/name")?;
                let email = string(&value, "/commit/author/email")?;
                Ok(format!("{name} <{email}>"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::config_path;

    #[test]
    fn config_paths_are_relative_to_the_root() {
        assert_eq!(
            config_path("/", "sanchoris.yaml").as_deref(),
            Some("sanchoris.yaml")
        );
        assert_eq!(
            config_path("/", "skills/lint/SKILL.md").as_deref(),
            Some("skills/lint/SKILL.md")
        );
        assert_eq!(config_path("/", "skills/lint/notes.md"), None);
        assert_eq!(config_path("/", "README.md"), None);
        assert_eq!(
            config_path("/ops/", "ops/sanchoris.yaml").as_deref(),
            Some("sanchoris.yaml")
        );
        assert_eq!(config_path("ops", "other/sanchoris.yaml"), None);
    }
}
