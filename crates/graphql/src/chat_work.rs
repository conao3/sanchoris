use std::collections::BTreeMap;

use async_graphql::{Enum, ID, Result, SimpleObject};
use chrono::{DateTime, Utc};
use sanchoris_core::db::TenantTx;
use sanchoris_core::items::{RunLogEntry, RunQuestion};
use sanchoris_core::keys;
use serde_json::Value;
use uuid::Uuid;

use crate::scalars::{Timestamp, ts};

const ASK_USER_QUESTION: &str = "AskUserQuestion";

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChatWorkKind {
    /// The agent's summarized reasoning.
    Thinking,
    /// Text the agent wrote between tool calls.
    Text,
    Tool,
    /// A question the agent asked, with the answer.
    Question,
}

#[derive(SimpleObject, Clone)]
pub(crate) struct ChatWork {
    pub kind: ChatWorkKind,
    pub at: Timestamp,
    /// Set for THINKING and TEXT.
    pub text: Option<String>,
    /// Set for TOOL.
    pub tool: Option<ChatTool>,
    /// Set for QUESTION.
    pub question: Option<ChatQuestion>,
}

#[derive(SimpleObject, Clone)]
pub(crate) struct ChatTool {
    /// The tool name as the agent reports it, such as Bash, Read, commandExecution or mcp__server__tool.
    pub name: String,
    /// The tool input as JSON.
    pub input: String,
    /// The output, truncated to 4000 characters. Null while the tool runs or when it reported nothing.
    pub output: Option<String>,
    pub is_error: bool,
}

#[derive(SimpleObject, Clone)]
pub(crate) struct ChatQuestion {
    pub id: ID,
    pub items: Vec<ChatQuestionItem>,
    /// Null until someone answers.
    pub answers: Option<Vec<ChatAnswer>>,
    pub asked_at: Timestamp,
}

#[derive(SimpleObject, Clone)]
pub(crate) struct ChatQuestionItem {
    pub id: String,
    pub header: String,
    pub question: String,
    pub multi_select: bool,
    pub options: Vec<ChatQuestionOption>,
}

#[derive(SimpleObject, Clone)]
pub(crate) struct ChatQuestionOption {
    pub label: String,
    pub description: String,
}

#[derive(SimpleObject, Clone)]
pub(crate) struct ChatAnswer {
    pub question_id: String,
    pub answers: Vec<String>,
}

/// The turn in progress of the conversation's task.
#[derive(SimpleObject, Clone)]
pub(crate) struct ChatWorking {
    pub started_at: Timestamp,
    pub work: Vec<ChatWork>,
    /// The question the agent waits on.
    pub question: Option<ChatQuestion>,
}

pub(crate) struct RunWork {
    entries: Vec<RunLogEntry>,
    questions: Vec<RunQuestion>,
}

impl RunWork {
    pub async fn load(tx: &mut TenantTx, run_id: Uuid) -> Result<Self> {
        let pk = keys::run_pk(tx.tenant_id(), run_id);
        let mut entries = tx.query::<RunLogEntry>(&pk, "LOG#").await?;
        entries.sort_by_key(|entry| entry.seq);
        let mut questions = tx.query::<RunQuestion>(&pk, "Q#").await?;
        questions.sort_by_key(|question| question.asked_at);
        Ok(Self { entries, questions })
    }

    pub fn first_at(&self, turn: i64) -> Option<DateTime<Utc>> {
        self.entries
            .iter()
            .find(|entry| entry.turn == turn)
            .map(|entry| entry.at)
    }

    pub fn open_question(&self, turn: i64) -> Result<Option<ChatQuestion>> {
        self.questions
            .iter()
            .rev()
            .find(|question| question.turn == turn && question.answers.is_none())
            .map(chat_question)
            .transpose()
    }

    /// The turn's work in order. `reply` drops the last TEXT when it repeats the reply.
    pub fn turn(&self, turn: i64, reply: Option<&str>) -> Result<Vec<ChatWork>> {
        let mut work: Vec<(DateTime<Utc>, ChatWork)> = Vec::new();
        let mut tools: BTreeMap<String, usize> = BTreeMap::new();
        for entry in self.entries.iter().filter(|entry| entry.turn == turn) {
            let data = &entry.data;
            let text = || data["text"].as_str().unwrap_or_default().to_string();
            match entry.kind.as_str() {
                "thinking" => {
                    work.push((entry.at, plain(ChatWorkKind::Thinking, entry.at, text())))
                }
                "text" if !text().trim().is_empty() => {
                    work.push((entry.at, plain(ChatWorkKind::Text, entry.at, text())))
                }
                "tool_use" => {
                    let name = data["name"].as_str().unwrap_or_default();
                    if name == ASK_USER_QUESTION {
                        continue;
                    }
                    if let Some(id) = data["id"].as_str() {
                        tools.insert(id.to_string(), work.len());
                    }
                    work.push((
                        entry.at,
                        ChatWork {
                            kind: ChatWorkKind::Tool,
                            at: ts(entry.at),
                            text: None,
                            tool: Some(ChatTool {
                                name: name.to_string(),
                                input: serde_json::to_string(&data["input"])?,
                                output: None,
                                is_error: false,
                            }),
                            question: None,
                        },
                    ));
                }
                "tool_result" => {
                    let Some(index) = data["toolUseId"]
                        .as_str()
                        .and_then(|id| tools.get(id).copied())
                    else {
                        continue;
                    };
                    if let Some(tool) = work[index].1.tool.as_mut() {
                        tool.output = data["content"]
                            .as_str()
                            .filter(|content| !content.is_empty())
                            .map(str::to_string);
                        tool.is_error = data["isError"].as_bool().unwrap_or(false);
                    }
                }
                _ => {}
            }
        }
        for question in self
            .questions
            .iter()
            .filter(|question| question.turn == turn && question.answers.is_some())
        {
            work.push((
                question.asked_at,
                ChatWork {
                    kind: ChatWorkKind::Question,
                    at: ts(question.asked_at),
                    text: None,
                    tool: None,
                    question: Some(chat_question(question)?),
                },
            ));
        }
        work.sort_by_key(|(at, _)| *at);
        let mut work: Vec<ChatWork> = work.into_iter().map(|(_, work)| work).collect();
        if let Some(reply) = reply
            && let Some(last) = work
                .iter()
                .rposition(|work| work.kind == ChatWorkKind::Text)
            && work[last].text.as_deref().map(str::trim) == Some(reply.trim())
        {
            work.remove(last);
        }
        Ok(work)
    }
}

fn plain(kind: ChatWorkKind, at: DateTime<Utc>, text: String) -> ChatWork {
    ChatWork {
        kind,
        at: ts(at),
        text: Some(text),
        tool: None,
        question: None,
    }
}

fn string(value: &Value, name: &str, what: &str) -> Result<String> {
    value[name]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| async_graphql::Error::new(format!("{what} has no {name}")))
}

fn chat_question(question: &RunQuestion) -> Result<ChatQuestion> {
    let what = format!("question {}", question.request_id);
    let items = question
        .questions
        .as_array()
        .ok_or_else(|| async_graphql::Error::new(format!("{what} has no questions")))?
        .iter()
        .map(|item| {
            Ok(ChatQuestionItem {
                id: string(item, "id", &what)?,
                header: string(item, "header", &what)?,
                question: string(item, "question", &what)?,
                multi_select: item["multiSelect"].as_bool().unwrap_or(false),
                options: item["options"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .map(|option| {
                        Ok(ChatQuestionOption {
                            label: string(option, "label", &what)?,
                            description: string(option, "description", &what)?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let answers = question
        .answers
        .as_ref()
        .map(|answers| {
            answers
                .as_object()
                .ok_or_else(|| async_graphql::Error::new(format!("{what} has malformed answers")))
                .map(|answers| {
                    answers
                        .iter()
                        .map(|(question_id, values)| ChatAnswer {
                            question_id: question_id.clone(),
                            answers: values
                                .as_array()
                                .map(Vec::as_slice)
                                .unwrap_or_default()
                                .iter()
                                .filter_map(|value| value.as_str().map(str::to_string))
                                .collect(),
                        })
                        .collect()
                })
        })
        .transpose()?;
    Ok(ChatQuestion {
        id: ID(question.request_id.clone()),
        items,
        answers,
        asked_at: ts(question.asked_at),
    })
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use serde_json::json;

    use super::*;

    fn entry(seq: i64, turn: i64, kind: &str, data: Value) -> RunLogEntry {
        let at = DateTime::from_timestamp(1_800_000_000 + seq, 0).unwrap();
        RunLogEntry {
            tenant_id: Uuid::nil(),
            run_id: Uuid::nil(),
            seq,
            turn,
            kind: kind.to_string(),
            data,
            at,
            expires_at: at + Duration::days(1),
        }
    }

    fn question(turn: i64, at_seq: i64, answers: Option<Value>) -> RunQuestion {
        RunQuestion {
            tenant_id: Uuid::nil(),
            run_id: Uuid::nil(),
            task_id: Uuid::nil(),
            request_id: format!("q-{turn}-{at_seq}"),
            turn,
            questions: json!([{
                "id": "Which label?", "header": "Label", "question": "Which label?",
                "multiSelect": true, "options": [{ "label": "bug", "description": "Bugs only" }],
            }]),
            answers,
            answered_by_member_id: None,
            asked_at: DateTime::from_timestamp(1_800_000_000 + at_seq, 0).unwrap(),
            answered_at: None,
        }
    }

    #[test]
    fn a_turn_pairs_tool_results_orders_answered_questions_and_drops_the_reply_text() {
        let work = RunWork {
            entries: vec![
                entry(1, 1, "thinking", json!({ "text": "Look first." })),
                entry(
                    2,
                    1,
                    "tool_use",
                    json!({ "id": "t1", "name": "Bash", "input": { "command": "ls" } }),
                ),
                entry(3, 1, "log", json!({ "line": "noise" })),
                entry(
                    4,
                    1,
                    "tool_use",
                    json!({ "id": "t2", "name": "AskUserQuestion", "input": {} }),
                ),
                entry(
                    5,
                    1,
                    "tool_result",
                    json!({ "toolUseId": "t1", "isError": true, "content": "boom" }),
                ),
                entry(7, 1, "text", json!({ "text": "Done." })),
                entry(8, 2, "text", json!({ "text": "Next turn." })),
            ],
            questions: vec![
                question(1, 6, Some(json!({ "Which label?": ["bug"] }))),
                question(2, 9, None),
            ],
        };
        let turn = work.turn(1, Some("Done.")).unwrap();
        let kinds: Vec<_> = turn.iter().map(|work| work.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ChatWorkKind::Thinking,
                ChatWorkKind::Tool,
                ChatWorkKind::Question
            ]
        );
        let tool = turn[1].tool.as_ref().unwrap();
        assert_eq!(tool.name, "Bash");
        assert_eq!(tool.input, r#"{"command":"ls"}"#);
        assert_eq!(
            (tool.output.as_deref(), tool.is_error),
            (Some("boom"), true)
        );
        let asked = turn[2].question.as_ref().unwrap();
        assert!(asked.items[0].multi_select);
        assert_eq!(asked.answers.as_ref().unwrap()[0].answers, vec!["bug"]);
        assert!(work.open_question(1).unwrap().is_none());
        assert_eq!(
            work.open_question(2).unwrap().unwrap().id,
            ID("q-2-9".into())
        );
        assert_eq!(
            work.turn(2, None).unwrap()[0].text.as_deref(),
            Some("Next turn.")
        );
    }
}
