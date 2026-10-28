use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

pub const CONFIG_REPO: &str = "acme/sanchoris-config";
pub const CONFIG_SHA: &str = "c0ff1ce0000000000000000000000000000000c1";
pub const CONFIG_REPO_ID: i64 = 7001;
pub const SHOP_WEB_ID: i64 = 7101;
pub const OPS_ID: i64 = 7301;

#[derive(Debug, Clone)]
pub struct Pull {
    pub repo: String,
    pub number: i64,
    pub head: String,
    pub base: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct Comment {
    pub id: i64,
    pub repo: String,
    pub number: i64,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct FileUpdate {
    pub repo: String,
    pub branch: String,
    pub path: String,
    pub text: String,
    pub previous_sha: Option<String>,
}

#[derive(Debug, Default)]
pub struct GitHubState {
    pub config_files: BTreeMap<String, String>,
    pub pulls: Vec<Pull>,
    pub comments: Vec<Comment>,
    pub checks: HashMap<String, String>,
    pub oauth_users: HashMap<String, (i64, String)>,
    pub user_installations: HashMap<String, Vec<Value>>,
    pub installation_repositories: HashMap<i64, Vec<(i64, String)>>,
    pub repositories: Vec<(i64, String)>,
    pub refs: HashMap<String, String>,
    pub file_updates: Vec<FileUpdate>,
    next_number: i64,
    next_comment: i64,
}

pub struct FakeGitHub {
    pub git_root: PathBuf,
    pub state: Mutex<GitHubState>,
}

fn not_found(what: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "message": format!("Not Found: {what}") })),
    )
        .into_response()
}

impl FakeGitHub {
    pub fn new(git_root: &Path, config_files: BTreeMap<String, String>) -> Arc<Self> {
        Arc::new(Self {
            git_root: git_root.to_path_buf(),
            state: Mutex::new(GitHubState {
                config_files,
                repositories: vec![
                    (CONFIG_REPO_ID, CONFIG_REPO.to_string()),
                    (SHOP_WEB_ID, "acme/shop-web".to_string()),
                    (OPS_ID, "acme/ops".to_string()),
                ],
                next_number: 52,
                next_comment: 9000,
                ..GitHubState::default()
            }),
        })
    }

    pub fn bare(&self, repo: &str) -> PathBuf {
        self.git_root.join(format!("{repo}.git"))
    }

    pub fn branch_head(&self, repo: &str, branch: &str) -> Option<String> {
        if let Some(sha) = self
            .state
            .lock()
            .unwrap()
            .refs
            .get(&format!("{repo}:{branch}"))
        {
            return Some(sha.clone());
        }
        if !self.bare(repo).exists() {
            return None;
        }
        let output = Command::new("git")
            .arg("--git-dir")
            .arg(self.bare(repo))
            .args([
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ])
            .output()
            .expect("git is on PATH");
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn set_check(&self, sha: &str, conclusion: &str) {
        self.state
            .lock()
            .unwrap()
            .checks
            .insert(sha.to_string(), conclusion.to_string());
    }

    pub fn pulls(&self) -> Vec<Pull> {
        self.state.lock().unwrap().pulls.clone()
    }

    pub fn comments(&self, repo: &str, number: i64) -> Vec<Comment> {
        self.state
            .lock()
            .unwrap()
            .comments
            .iter()
            .filter(|c| c.repo == repo && c.number == number)
            .cloned()
            .collect()
    }

    fn pull_json(&self, pull: &Pull) -> Value {
        let sha = self.branch_head(&pull.repo, &pull.head).unwrap_or_default();
        json!({
            "number": pull.number,
            "html_url": format!("https://github.com/{}/pull/{}", pull.repo, pull.number),
            "head": { "ref": pull.head, "sha": sha },
            "base": { "ref": pull.base },
            "state": "open",
            "merged": false,
            "title": pull.title,
        })
    }

    pub fn router(self: Arc<Self>) -> Router {
        Router::new()
            .route("/app/installations/{id}/access_tokens", post(access_token))
            .route("/repos/{owner}/{repo}", get(repository))
            .route("/repositories/{id}", get(repository_by_id))
            .route("/repositories/{id}/commits/{reference}", get(commit_by_id))
            .route("/repositories/{id}/git/trees/{sha}", get(tree))
            .route("/repositories/{id}/git/blobs/{sha}", get(blob_by_id))
            .route("/repos/{owner}/{repo}/commits/{reference}", get(commit))
            .route(
                "/repos/{owner}/{repo}/commits/{reference}/check-runs",
                get(check_runs),
            )
            .route(
                "/repos/{owner}/{repo}/contents/{*path}",
                get(contents).put(put_contents),
            )
            .route("/repos/{owner}/{repo}/git/refs", post(create_ref))
            .route("/user/installations", get(user_installations))
            .route("/installation/repositories", get(installation_repositories))
            .route("/repos/{owner}/{repo}/git/trees/{sha}", get(tree))
            .route("/repos/{owner}/{repo}/git/blobs/{sha}", get(blob))
            .route(
                "/repos/{owner}/{repo}/pulls",
                get(list_pulls).post(create_pull),
            )
            .route("/repos/{owner}/{repo}/pulls/{number}", get(get_pull))
            .route(
                "/repos/{owner}/{repo}/issues/{number}/comments",
                get(list_comments).post(create_comment),
            )
            .route(
                "/repos/{owner}/{repo}/issues/comments/{id}",
                patch(update_comment),
            )
            .route("/graphql", post(graphql))
            .route("/login/oauth/access_token", post(oauth_token))
            .route("/user", get(user))
            .with_state(self)
    }
}

type Fake = State<Arc<FakeGitHub>>;

async fn access_token(UrlPath(id): UrlPath<i64>) -> Json<Value> {
    Json(json!({ "token": format!("ghs_installation_{id}"), "expires_at": "2099-01-01T00:00:00Z" }))
}

fn repository_json(id: i64, full_name: &str) -> Response {
    Json(json!({ "id": id, "full_name": full_name, "default_branch": "main" })).into_response()
}

async fn repository(
    State(fake): Fake,
    UrlPath((owner, repo)): UrlPath<(String, String)>,
) -> Response {
    let full = format!("{owner}/{repo}");
    let known = fake
        .state
        .lock()
        .unwrap()
        .repositories
        .iter()
        .find(|(_, name)| name.eq_ignore_ascii_case(&full))
        .cloned();
    match known {
        Some((id, name)) => repository_json(id, &name),
        None => not_found(&full),
    }
}

fn name_of(fake: &FakeGitHub, id: i64) -> Option<String> {
    fake.state
        .lock()
        .unwrap()
        .repositories
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, name)| name.clone())
}

async fn repository_by_id(State(fake): Fake, UrlPath(id): UrlPath<i64>) -> Response {
    match name_of(&fake, id) {
        Some(name) => repository_json(id, &name),
        None => not_found(&id.to_string()),
    }
}

async fn commit_by_id(
    State(fake): Fake,
    UrlPath((id, reference)): UrlPath<(i64, String)>,
) -> Response {
    match name_of(&fake, id) {
        Some(full) => commit_of(&fake, &full, &reference),
        None => not_found(&id.to_string()),
    }
}

async fn blob_by_id(State(fake): Fake, UrlPath((_, sha)): UrlPath<(i64, String)>) -> Response {
    blob_of(&fake, &sha)
}

async fn commit(
    State(fake): Fake,
    UrlPath((owner, repo, reference)): UrlPath<(String, String, String)>,
) -> Response {
    commit_of(&fake, &format!("{owner}/{repo}"), &reference)
}

fn commit_of(fake: &FakeGitHub, full: &str, reference: &str) -> Response {
    let full = full.to_string();
    let reference = reference.to_string();
    if full == CONFIG_REPO {
        return Json(json!({
            "sha": CONFIG_SHA,
            "author": { "login": "mika-dev" },
            "commit": { "author": { "name": "Mika", "email": "mika@acme.test" } },
        }))
        .into_response();
    }
    match fake.branch_head(&full, &reference) {
        Some(sha) => Json(json!({ "sha": sha })).into_response(),
        None if reference.len() == 40 && reference.chars().all(|c| c.is_ascii_hexdigit()) => {
            Json(json!({ "sha": reference, "author": { "login": "mika-dev" } })).into_response()
        }
        None => not_found(&reference),
    }
}

async fn check_runs(
    State(fake): Fake,
    UrlPath((_, _, sha)): UrlPath<(String, String, String)>,
) -> Json<Value> {
    let conclusion = fake.state.lock().unwrap().checks.get(&sha).cloned();
    let run = match conclusion {
        Some(conclusion) => json!({
            "name": "ci/test", "status": "completed", "conclusion": conclusion,
            "html_url": format!("https://ci.example/{sha}"),
            "output": { "summary": if conclusion == "success" { Value::Null } else { json!("checkout_spec::flaky failed") }, "text": null },
        }),
        None => json!({
            "name": "ci/test", "status": "in_progress", "conclusion": null,
            "html_url": format!("https://ci.example/{sha}"), "output": { "summary": null, "text": null },
        }),
    };
    Json(json!({ "total_count": 1, "check_runs": [run] }))
}

fn blob_sha(index: usize) -> String {
    format!("{index:040x}")
}

async fn tree(State(fake): Fake) -> Json<Value> {
    let state = fake.state.lock().unwrap();
    let entries: Vec<Value> = state
        .config_files
        .keys()
        .enumerate()
        .map(|(index, path)| json!({ "path": path, "type": "blob", "sha": blob_sha(index) }))
        .collect();
    Json(json!({ "sha": CONFIG_SHA, "tree": entries, "truncated": false }))
}

async fn blob(
    State(fake): Fake,
    UrlPath((_, _, sha)): UrlPath<(String, String, String)>,
) -> Response {
    blob_of(&fake, &sha)
}

fn blob_of(fake: &FakeGitHub, sha: &str) -> Response {
    let sha = sha.to_string();
    let state = fake.state.lock().unwrap();
    match state
        .config_files
        .values()
        .enumerate()
        .find(|(index, _)| blob_sha(*index) == sha)
    {
        Some((_, content)) => Json(json!({
            "sha": sha,
            "encoding": "base64",
            "content": base64::engine::general_purpose::STANDARD.encode(content),
        }))
        .into_response(),
        None => not_found(&sha),
    }
}

#[derive(Deserialize)]
struct PullQuery {
    head: Option<String>,
}

async fn list_pulls(
    State(fake): Fake,
    UrlPath((owner, repo)): UrlPath<(String, String)>,
    Query(query): Query<PullQuery>,
) -> Json<Value> {
    let full = format!("{owner}/{repo}");
    let branch = query
        .head
        .as_deref()
        .and_then(|head| head.split_once(':'))
        .map(|(_, branch)| branch.to_string());
    let pulls = fake.pulls();
    let items: Vec<Value> = pulls
        .iter()
        .filter(|p| p.repo == full && branch.as_deref().is_none_or(|b| b == p.head))
        .map(|p| fake.pull_json(p))
        .collect();
    Json(Value::Array(items))
}

async fn create_pull(
    State(fake): Fake,
    UrlPath((owner, repo)): UrlPath<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let full = format!("{owner}/{repo}");
    let head = body["head"].as_str().unwrap_or_default().to_string();
    if fake.branch_head(&full, &head).is_none() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "message": "Validation Failed", "errors": [{ "message": format!("No commits between main and {head}") }] })),
        )
            .into_response();
    }
    let pull = {
        let mut state = fake.state.lock().unwrap();
        let number = state.next_number;
        state.next_number += 1;
        let pull = Pull {
            repo: full,
            number,
            head,
            base: body["base"].as_str().unwrap_or_default().to_string(),
            title: body["title"].as_str().unwrap_or_default().to_string(),
        };
        state.pulls.push(pull.clone());
        pull
    };
    (StatusCode::CREATED, Json(fake.pull_json(&pull))).into_response()
}

async fn get_pull(
    State(fake): Fake,
    UrlPath((owner, repo, number)): UrlPath<(String, String, i64)>,
) -> Response {
    let full = format!("{owner}/{repo}");
    match fake
        .pulls()
        .iter()
        .find(|p| p.repo == full && p.number == number)
    {
        Some(pull) => Json(fake.pull_json(pull)).into_response(),
        None => not_found("pull"),
    }
}

fn comment_json(comment: &Comment) -> Value {
    json!({
        "id": comment.id,
        "body": comment.body,
        "html_url": format!("https://github.com/{}/issues/{}#issuecomment-{}", comment.repo, comment.number, comment.id),
    })
}

async fn list_comments(
    State(fake): Fake,
    UrlPath((owner, repo, number)): UrlPath<(String, String, i64)>,
) -> Json<Value> {
    let comments = fake.comments(&format!("{owner}/{repo}"), number);
    Json(Value::Array(comments.iter().map(comment_json).collect()))
}

async fn create_comment(
    State(fake): Fake,
    UrlPath((owner, repo, number)): UrlPath<(String, String, i64)>,
    Json(body): Json<Value>,
) -> Response {
    let mut state = fake.state.lock().unwrap();
    let id = state.next_comment;
    state.next_comment += 1;
    let comment = Comment {
        id,
        repo: format!("{owner}/{repo}"),
        number,
        body: body["body"].as_str().unwrap_or_default().to_string(),
    };
    state.comments.push(comment.clone());
    (StatusCode::CREATED, Json(comment_json(&comment))).into_response()
}

async fn update_comment(
    State(fake): Fake,
    UrlPath((_, _, id)): UrlPath<(String, String, i64)>,
    Json(body): Json<Value>,
) -> Response {
    let mut state = fake.state.lock().unwrap();
    match state.comments.iter_mut().find(|c| c.id == id) {
        Some(comment) => {
            comment.body = body["body"].as_str().unwrap_or_default().to_string();
            Json(comment_json(comment)).into_response()
        }
        None => not_found("comment"),
    }
}

async fn graphql() -> Json<Value> {
    Json(
        json!({ "data": { "repository": { "issue": { "parent": null, "blockedBy": { "nodes": [] } } } } }),
    )
}

async fn oauth_token(State(fake): Fake, Json(body): Json<Value>) -> Json<Value> {
    let code = body["code"].as_str().unwrap_or_default();
    if fake.state.lock().unwrap().oauth_users.contains_key(code) {
        Json(json!({ "access_token": format!("gho_{code}"), "token_type": "bearer" }))
    } else {
        Json(json!({ "error": "bad_verification_code" }))
    }
}

async fn user(State(fake): Fake, headers: HeaderMap) -> Response {
    let code = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer gho_"))
        .map(str::to_string);
    let state = fake.state.lock().unwrap();
    match code.and_then(|code| state.oauth_users.get(&code).cloned()) {
        Some((id, login)) => Json(json!({ "id": id, "login": login })).into_response(),
        None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "message": "Bad credentials" })),
        )
            .into_response(),
    }
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
}

fn file_sha(text: &str) -> String {
    format!("{:040x}", text.len())
}

async fn contents(
    State(fake): Fake,
    UrlPath((_, _, path)): UrlPath<(String, String, String)>,
) -> Response {
    let state = fake.state.lock().unwrap();
    match state.config_files.get(&path) {
        Some(text) => Json(json!({
            "path": path,
            "sha": file_sha(text),
            "encoding": "base64",
            "content": base64::engine::general_purpose::STANDARD.encode(text),
        }))
        .into_response(),
        None => not_found(&path),
    }
}

async fn put_contents(
    State(fake): Fake,
    UrlPath((owner, repo, path)): UrlPath<(String, String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let text = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(body["content"].as_str().unwrap_or_default())
            .unwrap(),
    )
    .unwrap();
    let update = FileUpdate {
        repo: format!("{owner}/{repo}"),
        branch: body["branch"].as_str().unwrap_or_default().to_string(),
        path,
        text,
        previous_sha: body["sha"].as_str().map(str::to_string),
    };
    fake.state.lock().unwrap().file_updates.push(update);
    (
        StatusCode::CREATED,
        Json(json!({ "commit": { "sha": "f11e" } })),
    )
        .into_response()
}

async fn create_ref(
    State(fake): Fake,
    UrlPath((owner, repo)): UrlPath<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let reference = body["ref"].as_str().unwrap_or_default();
    let Some(branch) = reference.strip_prefix("refs/heads/") else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "message": "bad ref" })),
        )
            .into_response();
    };
    fake.state.lock().unwrap().refs.insert(
        format!("{owner}/{repo}:{branch}"),
        body["sha"].as_str().unwrap_or_default().to_string(),
    );
    (StatusCode::CREATED, Json(json!({ "ref": reference }))).into_response()
}

async fn user_installations(State(fake): Fake, headers: HeaderMap) -> Response {
    let state = fake.state.lock().unwrap();
    match bearer(&headers).and_then(|token| state.user_installations.get(&token).cloned()) {
        Some(installations) => Json(json!({
            "total_count": installations.len(),
            "installations": installations,
        }))
        .into_response(),
        None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "message": "Bad credentials" })),
        )
            .into_response(),
    }
}

async fn installation_repositories(State(fake): Fake, headers: HeaderMap) -> Response {
    let installation = bearer(&headers)
        .and_then(|token| token.strip_prefix("ghs_installation_").map(str::to_string))
        .and_then(|id| id.parse::<i64>().ok());
    let state = fake.state.lock().unwrap();
    match installation.and_then(|id| state.installation_repositories.get(&id).cloned()) {
        Some(repositories) => Json(json!({
            "total_count": repositories.len(),
            "repositories": repositories
                .iter()
                .map(|(id, name)| json!({ "id": id, "full_name": name }))
                .collect::<Vec<_>>(),
        }))
        .into_response(),
        None => not_found("installation"),
    }
}
