use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
#[error("invalid event pattern at {path}: {message}")]
pub struct PatternError {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventPattern {
    fields: Vec<(String, FieldPattern)>,
}

#[derive(Debug, Clone, PartialEq)]
enum FieldPattern {
    Nested(EventPattern),
    Leaf(Vec<Matcher>),
}

#[derive(Debug, Clone, PartialEq)]
enum Matcher {
    Equals(Value),
    Prefix(String),
    AnythingBut(AnythingBut),
    Exists(bool),
    Numeric(Vec<(NumericOp, f64)>),
}

#[derive(Debug, Clone, PartialEq)]
enum AnythingBut {
    Values(Vec<Value>),
    Prefix(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum NumericOp {
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
}

impl EventPattern {
    pub fn parse(pattern: &Value) -> Result<Self, PatternError> {
        parse_object(pattern, "$")
    }

    pub fn matches(&self, event: &Value) -> bool {
        self.matches_object(Some(event))
    }

    fn matches_object(&self, value: Option<&Value>) -> bool {
        self.fields.iter().all(|(key, field)| {
            let child = value.and_then(|v| v.as_object()).and_then(|o| o.get(key));
            match field {
                FieldPattern::Nested(nested) => match child {
                    Some(Value::Array(items)) => {
                        items.iter().any(|item| nested.matches_object(Some(item)))
                    }
                    Some(other) => nested.matches_object(Some(other)),
                    None => nested.matches_object(None),
                },
                FieldPattern::Leaf(matchers) => leaf_matches(matchers, child),
            }
        })
    }
}

fn error(path: &str, message: impl Into<String>) -> PatternError {
    PatternError {
        path: path.to_string(),
        message: message.into(),
    }
}

fn parse_object(value: &Value, path: &str) -> Result<EventPattern, PatternError> {
    let object = value
        .as_object()
        .ok_or_else(|| error(path, "expected an object"))?;
    let fields = object
        .iter()
        .map(|(key, child)| {
            let child_path = format!("{path}.{key}");
            let field = match child {
                Value::Object(_) => FieldPattern::Nested(parse_object(child, &child_path)?),
                Value::Array(items) => FieldPattern::Leaf(parse_leaf(items, &child_path)?),
                _ => return Err(error(&child_path, "expected an object or an array")),
            };
            Ok((key.clone(), field))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(EventPattern { fields })
}

fn parse_leaf(items: &[Value], path: &str) -> Result<Vec<Matcher>, PatternError> {
    if items.is_empty() {
        return Err(error(path, "expected at least one value"));
    }
    items
        .iter()
        .map(|item| match item {
            Value::Object(object) => parse_matcher(object, path),
            Value::Array(_) => Err(error(path, "nested arrays are not allowed")),
            scalar => Ok(Matcher::Equals(scalar.clone())),
        })
        .collect()
}

fn parse_matcher(object: &Map<String, Value>, path: &str) -> Result<Matcher, PatternError> {
    if object.len() != 1 {
        return Err(error(path, "a matcher must have exactly one key"));
    }
    let (key, value) = object.iter().next().expect("length checked");
    match key.as_str() {
        "prefix" => value
            .as_str()
            .map(|s| Matcher::Prefix(s.to_string()))
            .ok_or_else(|| error(path, "prefix must be a string")),
        "exists" => value
            .as_bool()
            .map(Matcher::Exists)
            .ok_or_else(|| error(path, "exists must be a boolean")),
        "anything-but" => parse_anything_but(value, path).map(Matcher::AnythingBut),
        "numeric" => parse_numeric(value, path).map(Matcher::Numeric),
        other => Err(error(path, format!("unknown matcher {other}"))),
    }
}

fn parse_anything_but(value: &Value, path: &str) -> Result<AnythingBut, PatternError> {
    match value {
        Value::Object(object) => {
            if object.len() != 1 {
                return Err(error(path, "anything-but object must have exactly one key"));
            }
            match object.get("prefix") {
                Some(Value::String(prefix)) => Ok(AnythingBut::Prefix(prefix.clone())),
                _ => Err(error(
                    path,
                    "anything-but object must be {\"prefix\": string}",
                )),
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                return Err(error(path, "anything-but list must not be empty"));
            }
            if items.iter().any(|item| item.is_array() || item.is_object()) {
                return Err(error(path, "anything-but list must contain scalars"));
            }
            Ok(AnythingBut::Values(items.clone()))
        }
        scalar => Ok(AnythingBut::Values(vec![scalar.clone()])),
    }
}

fn parse_numeric(value: &Value, path: &str) -> Result<Vec<(NumericOp, f64)>, PatternError> {
    let items = value
        .as_array()
        .ok_or_else(|| error(path, "numeric must be an array"))?;
    if items.len() != 2 && items.len() != 4 {
        return Err(error(path, "numeric must have one or two comparisons"));
    }
    let conditions = items
        .chunks(2)
        .map(|pair| {
            let op = match pair[0].as_str() {
                Some("=") => NumericOp::Eq,
                Some("<") => NumericOp::Lt,
                Some("<=") => NumericOp::Le,
                Some(">") => NumericOp::Gt,
                Some(">=") => NumericOp::Ge,
                _ => {
                    return Err(error(
                        path,
                        "numeric operator must be one of =, <, <=, >, >=",
                    ));
                }
            };
            let number = pair[1]
                .as_f64()
                .ok_or_else(|| error(path, "numeric operand must be a number"))?;
            Ok((op, number))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if conditions.len() == 2 && conditions.iter().any(|(op, _)| *op == NumericOp::Eq) {
        return Err(error(
            path,
            "numeric = cannot be combined with another comparison",
        ));
    }
    Ok(conditions)
}

fn leaf_matches(matchers: &[Matcher], value: Option<&Value>) -> bool {
    matchers.iter().any(|matcher| match (matcher, value) {
        (Matcher::Exists(expected), value) => *expected == value.is_some(),
        (_, None) => false,
        (matcher, Some(Value::Array(items))) => {
            items.iter().any(|item| scalar_matches(matcher, item))
        }
        (matcher, Some(value)) => scalar_matches(matcher, value),
    })
}

fn scalar_matches(matcher: &Matcher, value: &Value) -> bool {
    match matcher {
        Matcher::Equals(expected) => values_equal(expected, value),
        Matcher::Prefix(prefix) => value
            .as_str()
            .is_some_and(|s| s.starts_with(prefix.as_str())),
        Matcher::AnythingBut(AnythingBut::Values(excluded)) => !excluded
            .iter()
            .any(|candidate| values_equal(candidate, value)),
        Matcher::AnythingBut(AnythingBut::Prefix(prefix)) => value
            .as_str()
            .is_some_and(|s| !s.starts_with(prefix.as_str())),
        Matcher::Numeric(conditions) => value.as_f64().is_some_and(|number| {
            conditions.iter().all(|(op, operand)| match op {
                NumericOp::Eq => number == *operand,
                NumericOp::Lt => number < *operand,
                NumericOp::Le => number <= *operand,
                NumericOp::Gt => number > *operand,
                NumericOp::Ge => number >= *operand,
            })
        }),
        Matcher::Exists(_) => unreachable!("exists is handled by leaf_matches"),
    }
}

fn values_equal(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (a, b) => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn matches(pattern: Value, event: Value) -> bool {
        EventPattern::parse(&pattern).unwrap().matches(&event)
    }

    fn issue_event() -> Value {
        json!({
            "source": "github",
            "detail-type": "Issue Labeled",
            "detail": {
                "repository": "acme/shop-web",
                "label": "sanchoris",
                "issue": { "number": 35, "title": "Fix flaky checkout test", "labels": ["bug", "sanchoris"], "author_association": "MEMBER" },
                "sender": { "login": "mika-dev", "type": "User" },
                "author_association": "MEMBER"
            }
        })
    }

    #[test]
    fn exact_values_match_any_listed_value() {
        let pattern = json!({ "source": ["github"], "detail-type": ["Issue Labeled", "Issue Comment Created"] });
        assert!(matches(pattern.clone(), issue_event()));
        assert!(!matches(json!({ "source": ["slack"] }), issue_event()));
    }

    #[test]
    fn nested_objects_require_every_field() {
        let pattern = json!({
            "source": ["github"],
            "detail": {
                "repository": ["acme/shop-web"],
                "label": ["sanchoris"],
                "sender": { "login": ["mika-dev", "sanchoris[bot]"] }
            }
        });
        assert!(matches(pattern, issue_event()));
        let wrong_sender = json!({ "detail": { "sender": { "login": ["sota-dev"] } } });
        assert!(!matches(wrong_sender, issue_event()));
    }

    #[test]
    fn missing_fields_do_not_match_values() {
        assert!(!matches(
            json!({ "detail": { "comment": { "body": ["hi"] } } }),
            issue_event()
        ));
        assert!(!matches(json!({ "missing": ["x"] }), issue_event()));
    }

    #[test]
    fn arrays_in_events_match_if_any_element_matches() {
        assert!(matches(
            json!({ "detail": { "issue": { "labels": ["sanchoris"] } } }),
            issue_event()
        ));
        assert!(!matches(
            json!({ "detail": { "issue": { "labels": ["wontfix"] } } }),
            issue_event()
        ));
        let event = json!({ "items": [{ "kind": "a" }, { "kind": "b" }] });
        assert!(matches(
            json!({ "items": { "kind": ["b"] } }),
            event.clone()
        ));
        assert!(!matches(json!({ "items": { "kind": ["c"] } }), event));
    }

    #[test]
    fn prefix_matches_strings_only() {
        assert!(matches(
            json!({ "detail": { "repository": [{ "prefix": "acme/" }] } }),
            issue_event()
        ));
        assert!(!matches(
            json!({ "detail": { "repository": [{ "prefix": "other/" }] } }),
            issue_event()
        ));
        assert!(!matches(
            json!({ "detail": { "issue": { "number": [{ "prefix": "3" }] } } }),
            issue_event()
        ));
    }

    #[test]
    fn anything_but_excludes_values_and_prefixes() {
        assert!(matches(
            json!({ "detail": { "sender": { "login": [{ "anything-but": ["sota-dev"] }] } } }),
            issue_event()
        ));
        assert!(!matches(
            json!({ "detail": { "sender": { "login": [{ "anything-but": "mika-dev" }] } } }),
            issue_event()
        ));
        assert!(!matches(
            json!({ "detail": { "repository": [{ "anything-but": { "prefix": "acme/" } }] } }),
            issue_event()
        ));
        assert!(matches(
            json!({ "detail": { "repository": [{ "anything-but": { "prefix": "other/" } }] } }),
            issue_event()
        ));
        assert!(!matches(
            json!({ "detail": { "missing": [{ "anything-but": "x" }] } }),
            issue_event()
        ));
    }

    #[test]
    fn anything_but_on_arrays_matches_if_any_element_is_not_excluded() {
        assert!(matches(
            json!({ "detail": { "issue": { "labels": [{ "anything-but": "bug" }] } } }),
            issue_event()
        ));
        let event = json!({ "labels": ["bug"] });
        assert!(!matches(
            json!({ "labels": [{ "anything-but": "bug" }] }),
            event
        ));
    }

    #[test]
    fn exists_checks_presence() {
        assert!(matches(
            json!({ "detail": { "issue": { "number": [{ "exists": true }] } } }),
            issue_event()
        ));
        assert!(!matches(
            json!({ "detail": { "comment": [{ "exists": true }] } }),
            issue_event()
        ));
        assert!(matches(
            json!({ "detail": { "comment": [{ "exists": false }] } }),
            issue_event()
        ));
        assert!(!matches(
            json!({ "detail": { "label": [{ "exists": false }] } }),
            issue_event()
        ));
        assert!(matches(
            json!({ "detail": { "comment": { "body": [{ "exists": false }] } } }),
            issue_event()
        ));
    }

    #[test]
    fn exists_true_matches_null_values() {
        assert!(matches(
            json!({ "body": [{ "exists": true }] }),
            json!({ "body": null })
        ));
        assert!(matches(json!({ "body": [null] }), json!({ "body": null })));
    }

    #[test]
    fn numeric_comparisons() {
        let event = issue_event();
        assert!(matches(
            json!({ "detail": { "issue": { "number": [{ "numeric": [">", 30, "<=", 35] }] } } }),
            event.clone()
        ));
        assert!(!matches(
            json!({ "detail": { "issue": { "number": [{ "numeric": [">", 35] }] } } }),
            event.clone()
        ));
        assert!(matches(
            json!({ "detail": { "issue": { "number": [{ "numeric": ["=", 35] }] } } }),
            event.clone()
        ));
        assert!(matches(
            json!({ "detail": { "issue": { "number": [{ "numeric": [">=", 35, "<", 36] }] } } }),
            event.clone()
        ));
        assert!(!matches(
            json!({ "detail": { "issue": { "title": [{ "numeric": [">", 0] }] } } }),
            event
        ));
    }

    #[test]
    fn numbers_compare_by_value() {
        assert!(matches(json!({ "n": [5] }), json!({ "n": 5.0 })));
        assert!(!matches(json!({ "n": ["5"] }), json!({ "n": 5 })));
    }

    #[test]
    fn values_of_mixed_matchers_are_alternatives() {
        let pattern =
            json!({ "detail": { "sender": { "login": ["sota-dev", { "prefix": "mika" }] } } });
        assert!(matches(pattern, issue_event()));
    }

    #[test]
    fn empty_pattern_matches_everything() {
        assert!(matches(json!({}), issue_event()));
    }

    #[test]
    fn invalid_patterns_are_rejected() {
        let cases = [
            json!([]),
            json!({ "source": "github" }),
            json!({ "source": [] }),
            json!({ "source": [["github"]] }),
            json!({ "source": [{ "prefix": 1 }] }),
            json!({ "source": [{ "prefix": "a", "exists": true }] }),
            json!({ "source": [{ "suffix": "a" }] }),
            json!({ "n": [{ "numeric": [">"] }] }),
            json!({ "n": [{ "numeric": ["!=", 1] }] }),
            json!({ "n": [{ "numeric": ["=", 1, "<", 2] }] }),
            json!({ "n": [{ "numeric": [">", "1"] }] }),
            json!({ "n": [{ "exists": "yes" }] }),
            json!({ "n": [{ "anything-but": [] }] }),
            json!({ "n": [{ "anything-but": { "suffix": "x" } }] }),
        ];
        for case in cases {
            assert!(
                EventPattern::parse(&case).is_err(),
                "{case} should be rejected"
            );
        }
    }

    #[test]
    fn errors_report_the_path() {
        let err =
            EventPattern::parse(&json!({ "detail": { "sender": { "login": "x" } } })).unwrap_err();
        assert_eq!(err.path, "$.detail.sender.login");
    }
}
