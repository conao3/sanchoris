use serde_json::{Map, Value, json};
use thiserror::Error;

pub const ISSUE_LABELED: &str = "Issue Labeled";
pub const ISSUE_COMMENT_CREATED: &str = "Issue Comment Created";
pub const CHECK_RUN_COMPLETED: &str = "Check Run Completed";

#[derive(Debug, Error, PartialEq)]
pub enum GithubEventError {
    #[error("GitHub {event} payload is missing {pointer}")]
    MissingField { event: String, pointer: String },
    #[error("GitHub {event} payload has an unexpected type at {pointer}")]
    UnexpectedType { event: String, pointer: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedEvent {
    pub source: String,
    pub detail_type: String,
    pub detail: Value,
}

impl NormalizedEvent {
    pub fn to_value(&self) -> Value {
        json!({
            "source": self.source,
            "detail-type": self.detail_type,
            "detail": self.detail,
        })
    }
}

pub fn normalize_github_event(
    event_name: &str,
    payload: &Value,
) -> Result<Option<NormalizedEvent>, GithubEventError> {
    if !matches!(event_name, "issues" | "issue_comment" | "check_run") {
        return Ok(None);
    }
    let reader = Reader {
        event: event_name,
        payload,
    };
    let action = reader.string("/action")?;
    let normalized = match (event_name, action.as_str()) {
        ("issues", "labeled") => Some((
            ISSUE_LABELED,
            json!({
                "repository": reader.string("/repository/full_name")?,
                "installation": reader.installation()?,
                "issue": reader.issue()?,
                "label": reader.string("/label/name")?,
                "sender": reader.sender()?,
            }),
        )),
        ("issue_comment", "created") => Some((
            ISSUE_COMMENT_CREATED,
            json!({
                "repository": reader.string("/repository/full_name")?,
                "installation": reader.installation()?,
                "issue": reader.issue()?,
                "comment": {
                    "id": reader.integer("/comment/id")?,
                    "body": reader.string("/comment/body")?,
                    "url": reader.string("/comment/html_url")?,
                    "author_association": reader.string("/comment/author_association")?,
                    "user": { "login": reader.string("/comment/user/login")? },
                },
                "sender": reader.sender()?,
            }),
        )),
        ("check_run", "completed") => Some((
            CHECK_RUN_COMPLETED,
            json!({
                "repository": reader.string("/repository/full_name")?,
                "installation": reader.installation()?,
                "check_run": reader.check_run()?,
                "sender": reader.sender()?,
            }),
        )),
        _ => None,
    };
    Ok(normalized.map(|(detail_type, detail)| NormalizedEvent {
        source: "github".to_string(),
        detail_type: detail_type.to_string(),
        detail,
    }))
}

struct Reader<'a> {
    event: &'a str,
    payload: &'a Value,
}

impl Reader<'_> {
    fn get(&self, pointer: &str) -> Result<&Value, GithubEventError> {
        self.payload
            .pointer(pointer)
            .ok_or_else(|| GithubEventError::MissingField {
                event: self.event.to_string(),
                pointer: pointer.to_string(),
            })
    }

    fn unexpected(&self, pointer: &str) -> GithubEventError {
        GithubEventError::UnexpectedType {
            event: self.event.to_string(),
            pointer: pointer.to_string(),
        }
    }

    fn string(&self, pointer: &str) -> Result<String, GithubEventError> {
        self.get(pointer)?
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| self.unexpected(pointer))
    }

    fn nullable_string(&self, pointer: &str) -> Result<Value, GithubEventError> {
        match self.get(pointer)? {
            Value::Null => Ok(Value::Null),
            Value::String(s) => Ok(Value::String(s.clone())),
            _ => Err(self.unexpected(pointer)),
        }
    }

    fn integer(&self, pointer: &str) -> Result<i64, GithubEventError> {
        self.get(pointer)?
            .as_i64()
            .ok_or_else(|| self.unexpected(pointer))
    }

    fn array(&self, pointer: &str) -> Result<&Vec<Value>, GithubEventError> {
        self.get(pointer)?
            .as_array()
            .ok_or_else(|| self.unexpected(pointer))
    }

    fn installation(&self) -> Result<Value, GithubEventError> {
        Ok(json!({ "id": self.integer("/installation/id")? }))
    }

    fn sender(&self) -> Result<Value, GithubEventError> {
        Ok(json!({
            "login": self.string("/sender/login")?,
            "id": self.integer("/sender/id")?,
            "type": self.string("/sender/type")?,
        }))
    }

    fn issue(&self) -> Result<Value, GithubEventError> {
        let labels = self
            .array("/issue/labels")?
            .iter()
            .map(|label| {
                label
                    .get("name")
                    .and_then(Value::as_str)
                    .map(|name| Value::String(name.to_string()))
                    .ok_or_else(|| self.unexpected("/issue/labels"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut issue = Map::new();
        issue.insert("number".into(), json!(self.integer("/issue/number")?));
        issue.insert("title".into(), json!(self.string("/issue/title")?));
        issue.insert("body".into(), self.nullable_string("/issue/body")?);
        issue.insert("state".into(), json!(self.string("/issue/state")?));
        issue.insert("url".into(), json!(self.string("/issue/html_url")?));
        issue.insert(
            "author_association".into(),
            json!(self.string("/issue/author_association")?),
        );
        issue.insert(
            "user".into(),
            json!({ "login": self.string("/issue/user/login")? }),
        );
        issue.insert("labels".into(), Value::Array(labels));
        issue.insert(
            "is_pull_request".into(),
            json!(self.payload.pointer("/issue/pull_request").is_some()),
        );
        Ok(Value::Object(issue))
    }

    fn check_run(&self) -> Result<Value, GithubEventError> {
        let pull_requests = self
            .array("/check_run/pull_requests")?
            .iter()
            .map(|pr| {
                pr.get("number")
                    .and_then(Value::as_i64)
                    .map(|n| json!(n))
                    .ok_or_else(|| self.unexpected("/check_run/pull_requests"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({
            "id": self.integer("/check_run/id")?,
            "name": self.string("/check_run/name")?,
            "status": self.string("/check_run/status")?,
            "conclusion": self.nullable_string("/check_run/conclusion")?,
            "head_sha": self.string("/check_run/head_sha")?,
            "url": self.string("/check_run/html_url")?,
            "app": { "slug": self.string("/check_run/app/slug")? },
            "pull_requests": pull_requests,
        }))
    }
}
