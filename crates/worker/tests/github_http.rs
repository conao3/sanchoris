use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use base64::Engine;
use sanchoris_engine::{IdempotencyKey, Integration, Invocation, ResourceOutcome, ResourceRef};
use sanchoris_worker::github::transport::{HttpRequest, HttpResponse, HttpTransport};
use sanchoris_worker::github::{AppAuth, GitHubApi};
use sanchoris_worker::ports::{
    ConfigSource, GitHubError, GitHubReader, Installation, InvocationContext, IssueRef,
    ResourceInvoker,
};
use sanchoris_worker::resources::GitHubResources;
use serde_json::{Value, json};
use uuid::Uuid;

const BASE: &str = "https://github.test";

fn app_key() -> &'static str {
    static KEY: OnceLock<String> = OnceLock::new();
    KEY.get_or_init(|| {
        let output = std::process::Command::new("openssl")
            .args([
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
            ])
            .output()
            .expect("openssl is on PATH (run inside `nix develop`)");
        assert!(output.status.success(), "openssl genpkey failed");
        String::from_utf8(output.stdout).expect("PEM is UTF-8")
    })
}

struct Exchange {
    method: &'static str,
    path: String,
    body: Option<Value>,
    status: u16,
    response: Value,
}

fn exchange(method: &'static str, path: &str, status: u16, response: Value) -> Exchange {
    Exchange {
        method,
        path: path.to_string(),
        body: None,
        status,
        response,
    }
}

#[derive(Default)]
struct Recorded {
    script: Mutex<VecDeque<Exchange>>,
    seen: Mutex<Vec<HttpRequest>>,
}

impl Recorded {
    fn new(script: Vec<Exchange>) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(script.into()),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn assert_done(&self) {
        let left = self.script.lock().unwrap();
        assert!(
            left.is_empty(),
            "unused exchanges: {:?}",
            left.iter().map(|e| &e.path).collect::<Vec<_>>()
        );
    }
}

#[async_trait]
impl HttpTransport for Recorded {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, GitHubError> {
        let next = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| panic!("unexpected request {} {}", request.method, request.url));
        assert_eq!(
            (request.method, request.url.as_str()),
            (next.method, format!("{BASE}{}", next.path).as_str())
        );
        if let Some(body) = &next.body {
            assert_eq!(request.body.as_ref(), Some(body));
        }
        assert_eq!(
            request.header("Accept"),
            Some("application/vnd.github+json")
        );
        assert_eq!(request.header("X-GitHub-Api-Version"), Some("2022-11-28"));
        self.seen.lock().unwrap().push(request);
        Ok(HttpResponse {
            status: next.status,
            body: next.response.to_string(),
        })
    }
}

fn token_exchange() -> Exchange {
    exchange(
        "POST",
        "/app/installations/42/access_tokens",
        201,
        json!({ "token": "ghs_installation", "expires_at": "2099-01-01T00:00:00Z" }),
    )
}

fn api(script: Vec<Exchange>) -> (GitHubApi, Arc<Recorded>) {
    let recorded = Recorded::new(script);
    let api = GitHubApi::new(
        BASE,
        recorded.clone(),
        AppAuth::new("12345", app_key()).unwrap(),
    );
    (api, recorded)
}

fn pull(number: i64, sha: &str) -> Value {
    json!({
        "number": number,
        "html_url": format!("https://github.com/acme/shop-web/pull/{number}"),
        "head": { "ref": "sanchoris/task", "sha": sha },
        "state": "open",
        "merged_at": null,
    })
}

fn context() -> InvocationContext {
    InvocationContext {
        tenant_id: Uuid::nil(),
        task_id: Uuid::nil(),
        task_name: "acme/shop-web#35".into(),
        task_title: "Fix flaky checkout test (acme/shop-web#35)".into(),
        skill_run_id: Uuid::from_u128(7),
        skill_run_number: 1,
        state_name: "Report".into(),
        branch: "sanchoris/task".into(),
        installations: vec![Installation {
            id: 42,
            account_login: Some("acme".into()),
        }],
    }
}

fn invocation(action: &str, parameters: Value) -> Invocation {
    Invocation {
        resource: ResourceRef {
            service: "github".into(),
            action: action.into(),
            integration: Integration::RequestResponse,
        },
        parameters,
        idempotency_key: IdempotencyKey {
            skill_run_id: 1,
            sequence: 3,
            attempt: 1,
        },
        task_token: None,
    }
}

#[tokio::test]
async fn installation_tokens_come_from_an_app_jwt_and_are_cached() {
    let mut create = exchange("POST", "/repos/acme/shop-web/pulls", 201, pull(52, "abc"));
    create.body = Some(json!({
        "title": "Fix flaky checkout test (acme/shop-web#35)",
        "head": "sanchoris/task",
        "base": "main",
        "body": "Opened by sanchoris for task acme/shop-web#35.",
    }));
    let (api, recorded) = api(vec![
        token_exchange(),
        exchange(
            "GET",
            "/repos/acme/shop-web/pulls?state=open&head=acme:sanchoris/task",
            200,
            json!([]),
        ),
        exchange(
            "GET",
            "/repos/acme/shop-web",
            200,
            json!({ "default_branch": "main" }),
        ),
        create,
    ]);
    let resources = GitHubResources::new(Arc::new(api));
    let outcome = resources
        .invoke(
            &context(),
            &invocation("createPullRequests", json!({ "Repos": ["acme/shop-web"] })),
        )
        .await;
    assert_eq!(
        outcome,
        ResourceOutcome::Succeeded {
            output: json!([{ "Repo": "acme/shop-web", "Number": 52, "Url": "https://github.com/acme/shop-web/pull/52", "Created": true }])
        }
    );
    recorded.assert_done();
    let seen = recorded.seen.lock().unwrap();
    let jwt = seen[0]
        .header("Authorization")
        .unwrap()
        .strip_prefix("Bearer ")
        .unwrap();
    let parts = jwt.split('.').collect::<Vec<_>>();
    assert_eq!(parts.len(), 3);
    let claims: Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap(),
    )
    .unwrap();
    assert_eq!(claims["iss"], "12345");
    assert_eq!(
        claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
        600
    );
    for request in &seen[1..] {
        assert_eq!(
            request.header("Authorization"),
            Some("Bearer ghs_installation")
        );
    }
}

#[tokio::test]
async fn existing_pull_requests_are_reused() {
    let (api, recorded) = api(vec![
        token_exchange(),
        exchange(
            "GET",
            "/repos/acme/shop-web/pulls?state=open&head=acme:sanchoris/task",
            200,
            json!([pull(52, "abc")]),
        ),
    ]);
    let outcome = GitHubResources::new(Arc::new(api))
        .invoke(
            &context(),
            &invocation("createPullRequests", json!({ "Repos": ["acme/shop-web"] })),
        )
        .await;
    assert_eq!(
        outcome,
        ResourceOutcome::Succeeded {
            output: json!([{ "Repo": "acme/shop-web", "Number": 52, "Url": "https://github.com/acme/shop-web/pull/52", "Created": false }])
        }
    );
    recorded.assert_done();
}

#[tokio::test]
async fn update_mode_comments_edit_the_marked_comment() {
    let marker = format!("<!-- sanchoris:{}:Report -->", Uuid::from_u128(7));
    let mut update = exchange(
        "PATCH",
        "/repos/acme/shop-web/issues/comments/901",
        200,
        json!({ "id": 901, "body": "", "html_url": "https://github.com/acme/shop-web/issues/35#issuecomment-901" }),
    );
    update.body = Some(json!({ "body": format!("All green.\n\n{marker}") }));
    let (api, recorded) = api(vec![
        token_exchange(),
        exchange(
            "GET",
            "/repos/acme/shop-web/issues/35/comments?per_page=100&page=1",
            200,
            json!([
                { "id": 900, "body": "unrelated", "html_url": "https://github.com/x" },
                { "id": 901, "body": format!("Old text\n\n{marker}"), "html_url": "https://github.com/y" },
            ]),
        ),
        update,
    ]);
    let outcome = GitHubResources::new(Arc::new(api))
        .invoke(
            &context(),
            &invocation(
                "commentOnIssue",
                json!({ "Repo": "acme/shop-web", "Number": 35, "Body": "All green.", "Mode": "update" }),
            ),
        )
        .await;
    assert_eq!(
        outcome,
        ResourceOutcome::Succeeded {
            output: json!({ "Repo": "acme/shop-web", "Number": 35, "CommentId": 901, "Url": "https://github.com/acme/shop-web/issues/35#issuecomment-901" })
        }
    );
    recorded.assert_done();
}

#[tokio::test]
async fn github_errors_fail_the_resource() {
    let (api, recorded) = api(vec![
        token_exchange(),
        exchange("GET", "/repos/acme/shop-web/pulls/52", 200, pull(52, "abc")),
        exchange(
            "PUT",
            "/repos/acme/shop-web/pulls/52/merge",
            405,
            json!({ "message": "Pull Request is not mergeable" }),
        ),
    ]);
    let outcome = GitHubResources::new(Arc::new(api))
        .invoke(
            &context(),
            &invocation(
                "mergePullRequest",
                json!({ "Repo": "acme/shop-web", "Number": 52 }),
            ),
        )
        .await;
    let ResourceOutcome::Failed { error, cause } = outcome else {
        panic!("the merge fails");
    };
    assert_eq!(error.as_deref(), Some("GitHub.RequestFailed"));
    assert!(cause.unwrap().contains("405"));
    recorded.assert_done();
}

#[tokio::test]
async fn issue_relations_come_from_graphql() {
    let (api, recorded) = api(vec![
        token_exchange(),
        exchange(
            "POST",
            "/graphql",
            200,
            json!({ "data": { "repository": { "issue": {
                "parent": { "number": 10, "repository": { "nameWithOwner": "acme/idea" } },
                "blockedBy": { "nodes": [{ "number": 34, "repository": { "nameWithOwner": "acme/shop-web" } }] },
            } } } }),
        ),
    ]);
    let relations = api.issue_relations(42, "acme/shop-web", 35).await.unwrap();
    assert_eq!(
        relations.parent,
        Some(IssueRef {
            repository: "acme/idea".into(),
            number: 10
        })
    );
    assert_eq!(relations.blocked_by[0].task_name(), "acme/shop-web#34");
    let seen = recorded.seen.lock().unwrap();
    assert_eq!(
        seen[1].body.as_ref().unwrap()["variables"],
        json!({ "owner": "acme", "name": "shop-web", "number": 35 })
    );
}

#[tokio::test]
async fn config_files_are_read_from_the_tree_at_a_commit() {
    let encode = |text: &str| base64::engine::general_purpose::STANDARD.encode(text);
    let (api, recorded) = api(vec![
        token_exchange(),
        exchange(
            "GET",
            "/repos/acme/config",
            200,
            json!({ "default_branch": "main" }),
        ),
        exchange(
            "GET",
            "/repos/acme/config/commits/main",
            200,
            json!({ "sha": "c0ffee" }),
        ),
        exchange(
            "GET",
            "/repos/acme/config/git/trees/c0ffee?recursive=1",
            200,
            json!({ "truncated": false, "tree": [
                { "path": "README.md", "type": "blob", "sha": "r" },
                { "path": "sanchoris.yaml", "type": "blob", "sha": "y" },
                { "path": "skills", "type": "tree", "sha": "t" },
                { "path": "skills/lint/SKILL.md", "type": "blob", "sha": "m" },
            ] }),
        ),
        exchange(
            "GET",
            "/repos/acme/config/git/blobs/y",
            200,
            json!({ "encoding": "base64", "content": format!("{}\n", encode("Version: 1\n")) }),
        ),
        exchange(
            "GET",
            "/repos/acme/config/git/blobs/m",
            200,
            json!({ "encoding": "base64", "content": encode("---\nname: lint\n---\n") }),
        ),
    ]);
    let sha = api.head_commit(42, "acme/config").await.unwrap();
    assert_eq!(sha, "c0ffee");
    let files = api.fetch(42, "acme/config", "/", &sha).await.unwrap();
    assert_eq!(
        files.into_iter().collect::<Vec<_>>(),
        vec![
            ("sanchoris.yaml".to_string(), "Version: 1\n".to_string()),
            (
                "skills/lint/SKILL.md".to_string(),
                "---\nname: lint\n---\n".to_string()
            ),
        ]
    );
    recorded.assert_done();
}
