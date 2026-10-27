use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum TransformError {
    #[error("invalid input transformer: {0}")]
    InvalidTransformer(String),
    #[error("invalid path {path}: {message}")]
    InvalidPath { path: String, message: String },
    #[error("path {path} for <{name}> was not found in the event")]
    PathNotFound { name: String, path: String },
    #[error("placeholder <{0}> is not defined in InputPathsMap")]
    UnknownPlaceholder(String),
    #[error("rendered InputTemplate is not valid JSON: {0}")]
    InvalidTemplateOutput(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonPath {
    source: String,
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq)]
enum Segment {
    Key(String),
    Index(usize),
}

impl JsonPath {
    pub fn parse(path: &str) -> Result<Self, TransformError> {
        let invalid = |message: &str| TransformError::InvalidPath {
            path: path.to_string(),
            message: message.to_string(),
        };
        let mut rest = path
            .strip_prefix('$')
            .ok_or_else(|| invalid("must start with $"))?;
        let mut segments = Vec::new();
        while !rest.is_empty() {
            if let Some(after_dot) = rest.strip_prefix('.') {
                let end = after_dot.find(['.', '[']).unwrap_or(after_dot.len());
                if end == 0 {
                    return Err(invalid("empty key"));
                }
                segments.push(Segment::Key(after_dot[..end].to_string()));
                rest = &after_dot[end..];
            } else if let Some(after_bracket) = rest.strip_prefix('[') {
                let end = after_bracket
                    .find(']')
                    .ok_or_else(|| invalid("unclosed ["))?;
                let index = after_bracket[..end]
                    .parse::<usize>()
                    .map_err(|_| invalid("index must be a non-negative integer"))?;
                segments.push(Segment::Index(index));
                rest = &after_bracket[end + 1..];
            } else {
                return Err(invalid("expected . or ["));
            }
        }
        Ok(Self {
            source: path.to_string(),
            segments,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.source
    }

    pub fn select<'a>(&self, value: &'a Value) -> Option<&'a Value> {
        self.segments
            .iter()
            .try_fold(value, |current, segment| match segment {
                Segment::Key(key) => current.as_object()?.get(key),
                Segment::Index(index) => current.as_array()?.get(*index),
            })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputTransformer {
    paths: Vec<(String, JsonPath)>,
    template: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransformedInput {
    pub values: Map<String, Value>,
    pub input: Value,
}

impl InputTransformer {
    pub fn new(paths: Vec<(String, String)>, template: String) -> Result<Self, TransformError> {
        let paths = paths
            .into_iter()
            .map(|(name, path)| {
                if !is_placeholder_name(&name) {
                    return Err(TransformError::InvalidTransformer(format!(
                        "InputPathsMap key {name:?} must use letters, digits, _, - or ."
                    )));
                }
                Ok((name, JsonPath::parse(&path)?))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { paths, template })
    }

    pub fn from_value(value: &Value) -> Result<Self, TransformError> {
        let invalid = |message: &str| TransformError::InvalidTransformer(message.to_string());
        let object = value
            .as_object()
            .ok_or_else(|| invalid("must be an object"))?;
        let paths = object
            .get("InputPathsMap")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("InputPathsMap must be an object"))?
            .iter()
            .map(|(name, path)| {
                path.as_str()
                    .map(|p| (name.clone(), p.to_string()))
                    .ok_or_else(|| invalid("InputPathsMap values must be strings"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let template = object
            .get("InputTemplate")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("InputTemplate must be a string"))?;
        Self::new(paths, template.to_string())
    }

    pub fn extract(&self, event: &Value) -> Result<Map<String, Value>, TransformError> {
        self.paths
            .iter()
            .map(|(name, path)| {
                path.select(event)
                    .map(|value| (name.clone(), value.clone()))
                    .ok_or_else(|| TransformError::PathNotFound {
                        name: name.clone(),
                        path: path.as_str().to_string(),
                    })
            })
            .collect()
    }

    pub fn render_template(&self, values: &Map<String, Value>) -> Result<Value, TransformError> {
        let rendered = render_json_template(&self.template, values)?;
        serde_json::from_str(&rendered)
            .map_err(|error| TransformError::InvalidTemplateOutput(error.to_string()))
    }

    pub fn apply(&self, event: &Value) -> Result<TransformedInput, TransformError> {
        let values = self.extract(event)?;
        let input = self.render_template(&values)?;
        Ok(TransformedInput { values, input })
    }
}

pub fn render_instruction(
    instruction: &str,
    values: &Map<String, Value>,
) -> Result<String, TransformError> {
    let mut out = String::with_capacity(instruction.len());
    let mut rest = instruction;
    while let Some((before, name, after)) = next_placeholder(rest) {
        out.push_str(before);
        let value = values
            .get(name)
            .ok_or_else(|| TransformError::UnknownPlaceholder(name.to_string()))?;
        match value {
            Value::String(s) => out.push_str(s),
            other => out.push_str(&other.to_string()),
        }
        rest = after;
    }
    out.push_str(rest);
    Ok(out)
}

fn render_json_template(
    template: &str,
    values: &Map<String, Value>,
) -> Result<String, TransformError> {
    let mut out = String::with_capacity(template.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut rest = template;
    while let Some(ch) = rest.chars().next() {
        if ch == '<'
            && !escaped
            && let Some((_, name, after)) =
                next_placeholder(rest).filter(|(before, _, _)| before.is_empty())
        {
            let value = values
                .get(name)
                .ok_or_else(|| TransformError::UnknownPlaceholder(name.to_string()))?;
            if in_string {
                let text = match value {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let quoted = Value::String(text).to_string();
                out.push_str(&quoted[1..quoted.len() - 1]);
            } else {
                out.push_str(&value.to_string());
            }
            rest = after;
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
        }
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    Ok(out)
}

fn next_placeholder(text: &str) -> Option<(&str, &str, &str)> {
    let mut search_from = 0;
    while let Some(offset) = text[search_from..].find('<') {
        let start = search_from + offset;
        let after_open = &text[start + 1..];
        if let Some(end) = after_open.find('>') {
            let name = &after_open[..end];
            if is_placeholder_name(name) {
                return Some((&text[..start], name, &after_open[end + 1..]));
            }
        }
        search_from = start + 1;
    }
    None
}

fn is_placeholder_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event() -> Value {
        json!({
            "source": "github",
            "detail-type": "Issue Labeled",
            "detail": {
                "repository": "acme/shop-web",
                "issue": { "number": 35, "title": "Fix \"flaky\" checkout test", "labels": ["bug", "sanchoris"] }
            }
        })
    }

    fn transformer() -> InputTransformer {
        InputTransformer::from_value(&json!({
            "InputPathsMap": {
                "repository": "$.detail.repository",
                "number": "$.detail.issue.number",
                "title": "$.detail.issue.title"
            },
            "InputTemplate": "{ \"repository\": <repository>, \"number\": <number>, \"title\": <title> }"
        }))
        .unwrap()
    }

    #[test]
    fn json_path_selects_keys_and_indices() {
        let path = JsonPath::parse("$.detail.issue.labels[1]").unwrap();
        assert_eq!(path.select(&event()), Some(&json!("sanchoris")));
        assert_eq!(
            JsonPath::parse("$").unwrap().select(&json!(1)),
            Some(&json!(1))
        );
        assert_eq!(
            JsonPath::parse("$.detail-type").unwrap().select(&event()),
            Some(&json!("Issue Labeled"))
        );
        assert_eq!(
            JsonPath::parse("$.detail.nope").unwrap().select(&event()),
            None
        );
    }

    #[test]
    fn json_path_rejects_invalid_syntax() {
        for path in ["detail", "$.", "$..a", "$[x]", "$[1", "$a"] {
            assert!(JsonPath::parse(path).is_err(), "{path}");
        }
    }

    #[test]
    fn applies_the_template_with_json_values() {
        let result = transformer().apply(&event()).unwrap();
        assert_eq!(
            result.input,
            json!({ "repository": "acme/shop-web", "number": 35, "title": "Fix \"flaky\" checkout test" })
        );
        assert_eq!(result.values["number"], json!(35));
    }

    #[test]
    fn placeholders_inside_strings_are_inserted_as_text() {
        let transformer = InputTransformer::new(
            vec![
                ("number".into(), "$.detail.issue.number".into()),
                ("title".into(), "$.detail.issue.title".into()),
                ("labels".into(), "$.detail.issue.labels".into()),
            ],
            r##"{ "summary": "#<number>: <title>", "labels": <labels>, "literal": "a < b" }"##
                .into(),
        )
        .unwrap();
        let result = transformer.apply(&event()).unwrap();
        assert_eq!(
            result.input,
            json!({ "summary": "#35: Fix \"flaky\" checkout test", "labels": ["bug", "sanchoris"], "literal": "a < b" })
        );
    }

    #[test]
    fn missing_paths_fail() {
        let transformer = InputTransformer::new(
            vec![("x".into(), "$.detail.comment.body".into())],
            "<x>".into(),
        )
        .unwrap();
        assert_eq!(
            transformer.apply(&event()).unwrap_err(),
            TransformError::PathNotFound {
                name: "x".into(),
                path: "$.detail.comment.body".into()
            }
        );
    }

    #[test]
    fn unknown_placeholders_fail() {
        let transformer = InputTransformer::new(vec![], "{ \"a\": <b> }".into()).unwrap();
        assert_eq!(
            transformer.render_template(&Map::new()).unwrap_err(),
            TransformError::UnknownPlaceholder("b".into())
        );
    }

    #[test]
    fn invalid_template_output_fails() {
        let transformer = InputTransformer::new(
            vec![("n".into(), "$.detail.issue.number".into())],
            "{ n: <n> }".into(),
        )
        .unwrap();
        assert!(matches!(
            transformer.apply(&event()).unwrap_err(),
            TransformError::InvalidTemplateOutput(_)
        ));
    }

    #[test]
    fn transformer_shape_is_validated() {
        assert!(InputTransformer::from_value(&json!({ "InputTemplate": "{}" })).is_err());
        assert!(InputTransformer::from_value(&json!({ "InputPathsMap": {} })).is_err());
        assert!(
            InputTransformer::from_value(
                &json!({ "InputPathsMap": { "a": 1 }, "InputTemplate": "{}" })
            )
            .is_err()
        );
        assert!(
            InputTransformer::from_value(
                &json!({ "InputPathsMap": { "a b": "$.a" }, "InputTemplate": "{}" })
            )
            .is_err()
        );
    }

    #[test]
    fn renders_instruction_as_plain_text() {
        let values = transformer().extract(&event()).unwrap();
        let text = render_instruction(
            "Resolve issue <number> in <repository>: <title>. Write tests for the change.",
            &values,
        )
        .unwrap();
        assert_eq!(
            text,
            "Resolve issue 35 in acme/shop-web: Fix \"flaky\" checkout test. Write tests for the change."
        );
    }

    #[test]
    fn instruction_keeps_text_that_is_not_a_placeholder() {
        let mut values = Map::new();
        values.insert("text".into(), json!("hello"));
        assert_eq!(
            render_instruction("a < b <text> c<>d", &values).unwrap(),
            "a < b hello c<>d"
        );
        assert_eq!(
            render_instruction("<missing>", &values).unwrap_err(),
            TransformError::UnknownPlaceholder("missing".into())
        );
    }
}
