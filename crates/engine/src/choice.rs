use crate::path::{PathError, ReferencePath};
use crate::payload::EvalError;
use sanchoris_config::skill::{ChoiceRule, ChoiceState};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChoiceError {
    #[error("invalid choice rule: {0}")]
    Definition(String),
    #[error(transparent)]
    Eval(#[from] EvalError),
}

impl From<PathError> for ChoiceError {
    fn from(error: PathError) -> Self {
        ChoiceError::Eval(EvalError::Path(error))
    }
}

enum Comparison<'r> {
    StringEquals(&'r str),
    StringLessThan(&'r str),
    StringGreaterThan(&'r str),
    StringMatches(&'r str),
    NumericEquals(f64),
    NumericLessThan(f64),
    NumericLessThanEquals(f64),
    NumericGreaterThan(f64),
    NumericGreaterThanEquals(f64),
    BooleanEquals(bool),
    IsPresent(bool),
    IsNull(bool),
    IsString(bool),
    IsNumeric(bool),
    IsBoolean(bool),
}

fn comparisons(rule: &ChoiceRule) -> Vec<Comparison<'_>> {
    let mut out = Vec::new();
    out.extend(rule.string_equals.as_deref().map(Comparison::StringEquals));
    out.extend(
        rule.string_less_than
            .as_deref()
            .map(Comparison::StringLessThan),
    );
    out.extend(
        rule.string_greater_than
            .as_deref()
            .map(Comparison::StringGreaterThan),
    );
    out.extend(
        rule.string_matches
            .as_deref()
            .map(Comparison::StringMatches),
    );
    out.extend(rule.numeric_equals.map(Comparison::NumericEquals));
    out.extend(rule.numeric_less_than.map(Comparison::NumericLessThan));
    out.extend(
        rule.numeric_less_than_equals
            .map(Comparison::NumericLessThanEquals),
    );
    out.extend(
        rule.numeric_greater_than
            .map(Comparison::NumericGreaterThan),
    );
    out.extend(
        rule.numeric_greater_than_equals
            .map(Comparison::NumericGreaterThanEquals),
    );
    out.extend(rule.boolean_equals.map(Comparison::BooleanEquals));
    out.extend(rule.is_present.map(Comparison::IsPresent));
    out.extend(rule.is_null.map(Comparison::IsNull));
    out.extend(rule.is_string.map(Comparison::IsString));
    out.extend(rule.is_numeric.map(Comparison::IsNumeric));
    out.extend(rule.is_boolean.map(Comparison::IsBoolean));
    out
}

pub fn choose<'s>(
    state: &'s ChoiceState,
    input: &Value,
    context: &Value,
) -> Result<Option<&'s str>, ChoiceError> {
    for rule in &state.choices {
        let next = rule.next.as_deref().ok_or_else(|| {
            ChoiceError::Definition("a top-level choice rule needs Next".to_string())
        })?;
        if evaluate_rule(rule, input, context)? {
            return Ok(Some(next));
        }
    }
    Ok(state.default.as_deref())
}

pub fn evaluate_rule(
    rule: &ChoiceRule,
    input: &Value,
    context: &Value,
) -> Result<bool, ChoiceError> {
    let comparisons = comparisons(rule);
    let combinators = usize::from(rule.and.is_some())
        + usize::from(rule.or.is_some())
        + usize::from(rule.not.is_some());
    match (combinators, comparisons.len(), rule.variable.as_deref()) {
        (1, 0, None) => {
            if let Some(rules) = &rule.and {
                for r in rules {
                    if !evaluate_nested(r, input, context)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            } else if let Some(rules) = &rule.or {
                for r in rules {
                    if evaluate_nested(r, input, context)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            } else {
                let inner = rule.not.as_deref().expect("Not is present");
                Ok(!evaluate_nested(inner, input, context)?)
            }
        }
        (0, 1, Some(variable)) => {
            let selected = ReferencePath::parse(variable)?.select(input, context);
            compare(&comparisons[0], selected, variable)
        }
        _ => Err(ChoiceError::Definition(
            "a rule needs either one of And, Or, Not, or a Variable with exactly one comparison operator"
                .to_string(),
        )),
    }
}

fn evaluate_nested(rule: &ChoiceRule, input: &Value, context: &Value) -> Result<bool, ChoiceError> {
    if rule.next.is_some() {
        return Err(ChoiceError::Definition(
            "a rule nested in And, Or, or Not must not have Next".to_string(),
        ));
    }
    evaluate_rule(rule, input, context)
}

fn compare(
    comparison: &Comparison<'_>,
    selected: Option<&Value>,
    variable: &str,
) -> Result<bool, ChoiceError> {
    if let Comparison::IsPresent(expected) = comparison {
        return Ok(selected.is_some() == *expected);
    }
    let value = selected.ok_or_else(|| PathError::NotFound(variable.to_string()))?;
    let string = value.as_str();
    let number = value.as_f64();
    Ok(match comparison {
        Comparison::StringEquals(s) => string == Some(*s),
        Comparison::StringLessThan(s) => string.is_some_and(|v| v < *s),
        Comparison::StringGreaterThan(s) => string.is_some_and(|v| v > *s),
        Comparison::StringMatches(pattern) => string.is_some_and(|v| glob_matches(pattern, v)),
        Comparison::NumericEquals(n) => number == Some(*n),
        Comparison::NumericLessThan(n) => number.is_some_and(|v| v < *n),
        Comparison::NumericLessThanEquals(n) => number.is_some_and(|v| v <= *n),
        Comparison::NumericGreaterThan(n) => number.is_some_and(|v| v > *n),
        Comparison::NumericGreaterThanEquals(n) => number.is_some_and(|v| v >= *n),
        Comparison::BooleanEquals(b) => value.as_bool() == Some(*b),
        Comparison::IsNull(expected) => value.is_null() == *expected,
        Comparison::IsString(expected) => value.is_string() == *expected,
        Comparison::IsNumeric(expected) => value.is_number() == *expected,
        Comparison::IsBoolean(expected) => value.is_boolean() == *expected,
        Comparison::IsPresent(_) => unreachable!("handled above"),
    })
}

enum GlobToken {
    Literal(char),
    Star,
}

fn glob_matches(pattern: &str, text: &str) -> bool {
    let mut tokens = Vec::new();
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => tokens.push(GlobToken::Literal(chars.next().unwrap_or('\\'))),
            '*' => tokens.push(GlobToken::Star),
            other => tokens.push(GlobToken::Literal(other)),
        }
    }
    let text: Vec<char> = text.chars().collect();
    let mut reachable = vec![false; text.len() + 1];
    reachable[0] = true;
    for token in &tokens {
        let mut next = vec![false; text.len() + 1];
        match token {
            GlobToken::Star => {
                let mut seen = false;
                for i in 0..=text.len() {
                    seen |= reachable[i];
                    next[i] = seen;
                }
            }
            GlobToken::Literal(c) => {
                for i in 0..text.len() {
                    next[i + 1] = reachable[i] && text[i] == *c;
                }
            }
        }
        reachable = next;
    }
    reachable[text.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(value: Value) -> ChoiceRule {
        serde_json::from_value(value).unwrap()
    }

    fn eval(r: Value, input: Value) -> Result<bool, ChoiceError> {
        evaluate_rule(&rule(r), &input, &json!({}))
    }

    #[test]
    fn string_operators() {
        let input = json!({"s": "feature/login"});
        assert!(
            eval(
                json!({"Variable": "$.s", "StringEquals": "feature/login"}),
                input.clone()
            )
            .unwrap()
        );
        assert!(
            eval(
                json!({"Variable": "$.s", "StringLessThan": "g"}),
                input.clone()
            )
            .unwrap()
        );
        assert!(
            eval(
                json!({"Variable": "$.s", "StringGreaterThan": "a"}),
                input.clone()
            )
            .unwrap()
        );
        assert!(
            eval(
                json!({"Variable": "$.s", "StringMatches": "feature/*"}),
                input.clone()
            )
            .unwrap()
        );
        assert!(
            !eval(
                json!({"Variable": "$.s", "StringMatches": "fix/*"}),
                input.clone()
            )
            .unwrap()
        );
        assert!(
            eval(
                json!({"Variable": "$.s", "StringMatches": "*log*"}),
                input.clone()
            )
            .unwrap()
        );
        assert!(
            eval(
                json!({"Variable": "$.s", "StringMatches": "a\\*b"}),
                json!({"s": "a*b"})
            )
            .unwrap()
        );
        assert!(
            !eval(
                json!({"Variable": "$.s", "StringMatches": "a\\*b"}),
                json!({"s": "axb"})
            )
            .unwrap()
        );
        assert!(
            !eval(
                json!({"Variable": "$.s", "StringEquals": "1"}),
                json!({"s": 1})
            )
            .unwrap()
        );
    }

    #[test]
    fn numeric_and_boolean_operators() {
        let input = json!({"n": 3, "b": true});
        assert!(
            eval(
                json!({"Variable": "$.n", "NumericEquals": 3}),
                input.clone()
            )
            .unwrap()
        );
        assert!(
            eval(
                json!({"Variable": "$.n", "NumericLessThan": 4}),
                input.clone()
            )
            .unwrap()
        );
        assert!(
            eval(
                json!({"Variable": "$.n", "NumericLessThanEquals": 3}),
                input.clone()
            )
            .unwrap()
        );
        assert!(
            !eval(
                json!({"Variable": "$.n", "NumericGreaterThan": 3}),
                input.clone()
            )
            .unwrap()
        );
        assert!(
            eval(
                json!({"Variable": "$.n", "NumericGreaterThanEquals": 3}),
                input.clone()
            )
            .unwrap()
        );
        assert!(
            eval(
                json!({"Variable": "$.b", "BooleanEquals": true}),
                input.clone()
            )
            .unwrap()
        );
        assert!(!eval(json!({"Variable": "$.n", "BooleanEquals": true}), input).unwrap());
    }

    #[test]
    fn type_and_presence_operators() {
        let input = json!({"n": null, "s": "x", "i": 1.5, "b": false});
        assert!(
            eval(
                json!({"Variable": "$.missing", "IsPresent": false}),
                input.clone()
            )
            .unwrap()
        );
        assert!(eval(json!({"Variable": "$.n", "IsPresent": true}), input.clone()).unwrap());
        assert!(eval(json!({"Variable": "$.n", "IsNull": true}), input.clone()).unwrap());
        assert!(eval(json!({"Variable": "$.s", "IsString": true}), input.clone()).unwrap());
        assert!(eval(json!({"Variable": "$.i", "IsNumeric": true}), input.clone()).unwrap());
        assert!(eval(json!({"Variable": "$.b", "IsBoolean": true}), input.clone()).unwrap());
        assert!(
            eval(
                json!({"Variable": "$.s", "IsNumeric": false}),
                input.clone()
            )
            .unwrap()
        );
        assert!(matches!(
            eval(json!({"Variable": "$.missing", "IsNull": true}), input),
            Err(ChoiceError::Eval(_))
        ));
    }

    #[test]
    fn combinators() {
        let input = json!({"a": 1, "b": "x"});
        let and = json!({"And": [
            {"Variable": "$.a", "NumericEquals": 1},
            {"Variable": "$.b", "StringEquals": "x"}
        ]});
        assert!(eval(and, input.clone()).unwrap());
        let or = json!({"Or": [
            {"Variable": "$.a", "NumericEquals": 2},
            {"Not": {"Variable": "$.b", "StringEquals": "y"}}
        ]});
        assert!(eval(or, input.clone()).unwrap());
        let not = json!({"Not": {"Variable": "$.a", "NumericEquals": 1}});
        assert!(!eval(not, input).unwrap());
    }

    #[test]
    fn invalid_rules_fail() {
        let input = json!({"a": 1});
        assert!(matches!(
            eval(json!({"Variable": "$.a"}), input.clone()),
            Err(ChoiceError::Definition(_))
        ));
        assert!(matches!(
            eval(
                json!({"Variable": "$.a", "NumericEquals": 1, "NumericLessThan": 2}),
                input.clone()
            ),
            Err(ChoiceError::Definition(_))
        ));
        assert!(matches!(
            eval(
                json!({"Not": {"Variable": "$.a", "NumericEquals": 1, "Next": "X"}}),
                input
            ),
            Err(ChoiceError::Definition(_))
        ));
    }

    #[test]
    fn choose_uses_first_match_then_default() {
        let state: ChoiceState = serde_json::from_value(json!({
            "Choices": [
                {"Variable": "$.c", "StringEquals": "success", "Next": "Report"},
                {"Variable": "$.n", "NumericGreaterThanEquals": 3, "Next": "GiveUp"}
            ],
            "Default": "HandOff"
        }))
        .unwrap();
        let ctx = json!({});
        assert_eq!(
            choose(&state, &json!({"c": "success", "n": 5}), &ctx).unwrap(),
            Some("Report")
        );
        assert_eq!(
            choose(&state, &json!({"c": "failure", "n": 5}), &ctx).unwrap(),
            Some("GiveUp")
        );
        assert_eq!(
            choose(&state, &json!({"c": "failure", "n": 1}), &ctx).unwrap(),
            Some("HandOff")
        );
        let no_default = ChoiceState {
            default: None,
            ..state
        };
        assert_eq!(
            choose(&no_default, &json!({"c": "failure", "n": 1}), &ctx).unwrap(),
            None
        );
    }
}
