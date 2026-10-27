use crate::intrinsic::{self, IntrinsicError};
use crate::path::{PathError, select_required};
use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EvalError {
    #[error(transparent)]
    Path(#[from] PathError),
    #[error(transparent)]
    Intrinsic(#[from] IntrinsicError),
    #[error("{0}")]
    Template(String),
}

pub fn build(template: &Value, input: &Value, context: &Value) -> Result<Value, EvalError> {
    match template {
        Value::Object(fields) => {
            let mut out = Map::with_capacity(fields.len());
            for (key, value) in fields {
                match key.strip_suffix(".$") {
                    Some(name) => {
                        let Value::String(expr) = value else {
                            return Err(EvalError::Template(format!(
                                "the value of {key:?} must be a path or an intrinsic call string"
                            )));
                        };
                        out.insert(name.to_string(), resolve(expr, input, context)?);
                    }
                    None => {
                        out.insert(key.clone(), build(value, input, context)?);
                    }
                }
            }
            Ok(Value::Object(out))
        }
        Value::Array(items) => items
            .iter()
            .map(|item| build(item, input, context))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        other => Ok(other.clone()),
    }
}

pub fn resolve(expr: &str, input: &Value, context: &Value) -> Result<Value, EvalError> {
    if expr.starts_with('$') {
        Ok(select_required(expr, input, context)?.clone())
    } else if expr.starts_with("States.") {
        Ok(intrinsic::evaluate(expr, input, context)?)
    } else {
        Err(EvalError::Template(format!(
            "{expr:?} is neither a path nor an intrinsic call"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_nested_templates() {
        let template = json!({
            "Repo": "acme/shop-web",
            "Repos.$": "$.Repos",
            "Token.$": "$$.Task.Token",
            "Nested": {"Count.$": "States.MathAdd($.n, 1)", "List": [{"X.$": "$.n"}]}
        });
        let input = json!({"Repos": ["a/b"], "n": 2});
        let context = json!({"Task": {"Token": "t"}});
        assert_eq!(
            build(&template, &input, &context).unwrap(),
            json!({
                "Repo": "acme/shop-web",
                "Repos": ["a/b"],
                "Token": "t",
                "Nested": {"Count": 3, "List": [{"X": 2}]}
            })
        );
    }

    #[test]
    fn missing_paths_and_bad_values_fail() {
        let context = json!({});
        assert!(matches!(
            build(&json!({"A.$": "$.missing"}), &json!({}), &context),
            Err(EvalError::Path(PathError::NotFound(_)))
        ));
        assert!(matches!(
            build(&json!({"A.$": 1}), &json!({}), &context),
            Err(EvalError::Template(_))
        ));
        assert!(matches!(
            build(&json!({"A.$": "plain"}), &json!({}), &context),
            Err(EvalError::Template(_))
        ));
    }
}
