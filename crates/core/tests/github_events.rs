use sanchoris_core::github::{GithubEventError, normalize_github_event};
use sanchoris_core::pattern::EventPattern;
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/tests/fixtures/github/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn issue_labeled_is_normalized() {
    let event = normalize_github_event("issues", &fixture("issues-labeled.json"))
        .unwrap()
        .unwrap();
    assert_eq!(
        event.to_value(),
        json!({
            "source": "github",
            "detail-type": "Issue Labeled",
            "detail": {
                "repository": "acme/shop-web",
                "repository_id": 500,
                "installation": { "id": 7001 },
                "issue": {
                    "number": 35,
                    "title": "Fix flaky checkout test",
                    "body": "checkout_spec fails about once in ten runs.",
                    "state": "open",
                    "url": "https://github.com/acme/shop-web/issues/35",
                    "author_association": "MEMBER",
                    "user": { "login": "sota-dev" },
                    "labels": ["bug", "sanchoris"],
                    "is_pull_request": false
                },
                "label": "sanchoris",
                "sender": { "login": "mika-dev", "id": 101, "type": "User" }
            }
        })
    );
}

#[test]
fn issue_comment_created_is_normalized() {
    let event = normalize_github_event("issue_comment", &fixture("issue-comment-created.json"))
        .unwrap()
        .unwrap();
    let value = event.to_value();
    assert_eq!(value["detail-type"], "Issue Comment Created");
    assert_eq!(
        value["detail"]["comment"],
        json!({
            "id": 9001,
            "body": "@sanchoris please also update the README",
            "url": "https://github.com/acme/shop-web/pull/52#issuecomment-9001",
            "author_association": "MEMBER",
            "user": { "login": "mika-dev" }
        })
    );
    assert_eq!(value["detail"]["issue"]["body"], Value::Null);
    assert_eq!(value["detail"]["issue"]["is_pull_request"], true);
    assert_eq!(value["detail"]["issue"]["author_association"], "NONE");
    assert_eq!(value["detail"]["author_association"], Value::Null);
    assert_eq!(value["detail"]["sender"]["login"], "mika-dev");
}

#[test]
fn check_run_completed_is_normalized() {
    let event = normalize_github_event("check_run", &fixture("check-run-completed.json"))
        .unwrap()
        .unwrap();
    assert_eq!(
        event.to_value()["detail"],
        json!({
            "repository": "acme/shop-web",
            "repository_id": 500,
            "installation": { "id": 7001 },
            "check_run": {
                "id": 4001,
                "name": "test",
                "status": "completed",
                "conclusion": "failure",
                "head_sha": "0123456789abcdef0123456789abcdef01234567",
                "url": "https://github.com/acme/shop-web/runs/4001",
                "app": { "slug": "github-actions" },
                "pull_requests": [52]
            },
            "sender": { "login": "github-actions[bot]", "id": 41898282, "type": "Bot" }
        })
    );
}

#[test]
fn other_events_and_actions_are_ignored() {
    let mut unlabeled = fixture("issues-labeled.json");
    unlabeled["action"] = json!("unlabeled");
    assert_eq!(normalize_github_event("issues", &unlabeled).unwrap(), None);
    assert_eq!(
        normalize_github_event("push", &json!({ "ref": "refs/heads/main" })).unwrap(),
        None
    );
    let mut edited = fixture("issue-comment-created.json");
    edited["action"] = json!("edited");
    assert_eq!(
        normalize_github_event("issue_comment", &edited).unwrap(),
        None
    );
}

#[test]
fn malformed_payloads_fail() {
    let mut payload = fixture("issues-labeled.json");
    payload["issue"]
        .as_object_mut()
        .unwrap()
        .remove("author_association");
    assert_eq!(
        normalize_github_event("issues", &payload).unwrap_err(),
        GithubEventError::MissingField {
            event: "issues".into(),
            pointer: "/issue/author_association".into()
        }
    );
    let mut payload = fixture("issues-labeled.json");
    payload["issue"]["number"] = json!("35");
    assert_eq!(
        normalize_github_event("issues", &payload).unwrap_err(),
        GithubEventError::UnexpectedType {
            event: "issues".into(),
            pointer: "/issue/number".into()
        }
    );
}

#[test]
fn normalized_events_match_the_labeled_issue_trigger() {
    let pattern = EventPattern::parse(&json!({
        "source": ["github"],
        "detail-type": ["Issue Labeled"],
        "detail": {
            "repository": ["acme/shop-web", "acme/shop-api"],
            "label": ["sanchoris"],
            "sender": { "login": ["mika-dev", "sota-dev", "sanchoris[bot]"] }
        }
    }))
    .unwrap();
    let labeled = normalize_github_event("issues", &fixture("issues-labeled.json"))
        .unwrap()
        .unwrap();
    assert!(pattern.matches(&labeled.to_value()));
    let comment = normalize_github_event("issue_comment", &fixture("issue-comment-created.json"))
        .unwrap()
        .unwrap();
    assert!(!pattern.matches(&comment.to_value()));
}
