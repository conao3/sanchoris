use crate::path::{PathError, ReferencePath};
use serde_json::{Number, Value};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Call { name: String, args: Vec<Expr> },
    Str { raw: String },
    Number(Number),
    Path(ReferencePath),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IntrinsicError {
    #[error("cannot parse intrinsic {source_text:?} at offset {offset}: {reason}")]
    Parse {
        source_text: String,
        offset: usize,
        reason: String,
    },
    #[error("{function}: {reason}")]
    Call { function: String, reason: String },
    #[error(transparent)]
    Path(#[from] PathError),
}

pub fn parse(source: &str) -> Result<Expr, IntrinsicError> {
    let mut parser = Parser {
        source,
        chars: source.char_indices().collect(),
        pos: 0,
    };
    parser.skip_ws();
    let expr = parser.call()?;
    parser.skip_ws();
    if parser.pos != parser.chars.len() {
        return Err(parser.error("unexpected trailing input"));
    }
    Ok(expr)
}

pub fn evaluate(source: &str, input: &Value, context: &Value) -> Result<Value, IntrinsicError> {
    eval(&parse(source)?, input, context)
}

struct Parser<'s> {
    source: &'s str,
    chars: Vec<(usize, char)>,
    pos: usize,
}

impl Parser<'_> {
    fn error(&self, reason: &str) -> IntrinsicError {
        IntrinsicError::Parse {
            source_text: self.source.to_string(),
            offset: self
                .chars
                .get(self.pos)
                .map_or(self.source.len(), |(offset, _)| *offset),
            reason: reason.to_string(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).map(|(_, c)| *c)
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), IntrinsicError> {
        if self.peek() == Some(expected) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected {expected:?}")))
        }
    }

    fn call(&mut self) -> Result<Expr, IntrinsicError> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '.')
        {
            self.pos += 1;
        }
        let name: String = self.chars[start..self.pos].iter().map(|(_, c)| c).collect();
        if !name.starts_with("States.") || name.len() == "States.".len() {
            self.pos = start;
            return Err(self.error("expected an intrinsic function name like States.Format"));
        }
        self.skip_ws();
        self.expect('(')?;
        let mut args = Vec::new();
        self.skip_ws();
        if self.peek() == Some(')') {
            self.pos += 1;
            return Ok(Expr::Call { name, args });
        }
        loop {
            self.skip_ws();
            args.push(self.argument()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some(')') => {
                    self.pos += 1;
                    return Ok(Expr::Call { name, args });
                }
                _ => return Err(self.error("expected , or )")),
            }
        }
    }

    fn argument(&mut self) -> Result<Expr, IntrinsicError> {
        match self.peek() {
            Some('\'') => self.string(),
            Some('$') => self.path(),
            Some(c) if c == '-' || c.is_ascii_digit() => self.number(),
            Some('S') => self.call(),
            _ => Err(self.error("expected a string, number, path, or intrinsic call")),
        }
    }

    fn string(&mut self) -> Result<Expr, IntrinsicError> {
        self.expect('\'')?;
        let mut raw = String::new();
        loop {
            match self.peek() {
                None => return Err(self.error("unterminated string literal")),
                Some('\\') => {
                    raw.push('\\');
                    self.pos += 1;
                    match self.peek() {
                        Some(c @ ('\'' | '{' | '}' | '\\')) => {
                            raw.push(c);
                            self.pos += 1;
                        }
                        _ => return Err(self.error("invalid escape sequence")),
                    }
                }
                Some('\'') => {
                    self.pos += 1;
                    return Ok(Expr::Str { raw });
                }
                Some(c) => {
                    raw.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    fn number(&mut self) -> Result<Expr, IntrinsicError> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))
        {
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().map(|(_, c)| c).collect();
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Number(n)) => Ok(Expr::Number(n)),
            _ => {
                self.pos = start;
                Err(self.error("invalid number"))
            }
        }
    }

    fn path(&mut self) -> Result<Expr, IntrinsicError> {
        let start = self.pos;
        let mut quote = None;
        let mut depth = 0usize;
        while let Some(c) = self.peek() {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {}
                (None, '\'' | '"') if depth > 0 => quote = Some(c),
                (None, '[') => depth += 1,
                (None, ']') => depth = depth.saturating_sub(1),
                (None, ',' | ')') if depth == 0 => break,
                (None, c) if c.is_whitespace() && depth == 0 => break,
                _ => {}
            }
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().map(|(_, c)| c).collect();
        Ok(Expr::Path(ReferencePath::parse(&text)?))
    }
}

fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.extend(chars.next());
        } else {
            out.push(c);
        }
    }
    out
}

fn eval(expr: &Expr, input: &Value, context: &Value) -> Result<Value, IntrinsicError> {
    match expr {
        Expr::Str { raw } => Ok(Value::String(unescape(raw))),
        Expr::Number(n) => Ok(Value::Number(n.clone())),
        Expr::Path(path) => path
            .select(input, context)
            .cloned()
            .ok_or_else(|| PathError::NotFound(path.to_string()).into()),
        Expr::Call { name, args } => call(name, args, input, context),
    }
}

fn call(
    name: &str,
    args: &[Expr],
    input: &Value,
    context: &Value,
) -> Result<Value, IntrinsicError> {
    let fail = |reason: String| IntrinsicError::Call {
        function: name.to_string(),
        reason,
    };
    let arity = |n: usize| {
        if args.len() == n {
            Ok(())
        } else {
            Err(fail(format!("expects {n} arguments, got {}", args.len())))
        }
    };
    let values = || {
        args.iter()
            .map(|a| eval(a, input, context))
            .collect::<Result<Vec<_>, _>>()
    };
    match name {
        "States.Format" => {
            let (template, rest) = args
                .split_first()
                .ok_or_else(|| fail("expects a template argument".to_string()))?;
            let template_raw = match template {
                Expr::Str { raw } => raw.clone(),
                other => match eval(other, input, context)? {
                    Value::String(s) => escape(&s),
                    _ => return Err(fail("the template must be a string".to_string())),
                },
            };
            let values = rest
                .iter()
                .map(|a| eval(a, input, context))
                .collect::<Result<Vec<_>, _>>()?;
            format(&template_raw, &values)
                .map_err(fail)
                .map(Value::String)
        }
        "States.JsonToString" => {
            arity(1)?;
            let value = values()?.remove(0);
            Ok(Value::String(value.to_string()))
        }
        "States.StringToJson" => {
            arity(1)?;
            match values()?.remove(0) {
                Value::String(s) => serde_json::from_str(&s)
                    .map_err(|e| fail(format!("the argument is not valid JSON: {e}"))),
                _ => Err(fail("the argument must be a string".to_string())),
            }
        }
        "States.MathAdd" => {
            arity(2)?;
            let values = values()?;
            let int = |v: &Value| {
                v.as_i64()
                    .ok_or_else(|| fail(format!("the argument {v} is not an integer")))
            };
            let sum = int(&values[0])?
                .checked_add(int(&values[1])?)
                .ok_or_else(|| fail("the sum overflows".to_string()))?;
            Ok(Value::from(sum))
        }
        other => Err(fail(format!(
            "{other} is not a supported intrinsic function"
        ))),
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\'' | '{' | '}' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn format(template_raw: &str, values: &[Value]) -> Result<String, String> {
    let mut out = String::new();
    let mut remaining = values.iter();
    let mut used = 0;
    let mut chars = template_raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            '{' => {
                if chars.next() != Some('}') {
                    return Err("an unescaped { must be followed by }".to_string());
                }
                let value = remaining.next().ok_or_else(|| {
                    format!(
                        "the template has more placeholders than the {} arguments",
                        values.len()
                    )
                })?;
                used += 1;
                match value {
                    Value::String(s) => out.push_str(s),
                    other => out.push_str(&other.to_string()),
                }
            }
            '}' => return Err("an unescaped } has no matching {".to_string()),
            other => out.push(other),
        }
    }
    if used != values.len() {
        return Err(format!(
            "the template has {used} placeholders but {} arguments were given",
            values.len()
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(source: &str, input: Value) -> Result<Value, IntrinsicError> {
        evaluate(source, &input, &json!({"Task": {"Token": "tok"}}))
    }

    #[test]
    fn format_with_paths_and_values() {
        let input = json!({"checks": {"Failed": "a#1", "Log": "boom"}, "prs": [{"n": 1}]});
        assert_eq!(
            run(
                "States.Format('CI failed on {}. Log: {}', $.checks.Failed, $.checks.Log)",
                input.clone()
            )
            .unwrap(),
            json!("CI failed on a#1. Log: boom")
        );
        assert_eq!(
            run("States.Format('PRs: {}', $.prs)", input.clone()).unwrap(),
            json!(r#"PRs: [{"n":1}]"#)
        );
        assert_eq!(
            run(
                r"States.Format('it\'s \{literal\} {} {}', 3, $$.Task.Token)",
                input
            )
            .unwrap(),
            json!("it's {literal} 3 tok")
        );
    }

    #[test]
    fn format_rejects_mismatched_arguments() {
        assert!(run("States.Format('{} {}', 1)", json!({})).is_err());
        assert!(run("States.Format('{}', 1, 2)", json!({})).is_err());
        assert!(run("States.Format('{', 1)", json!({})).is_err());
    }

    #[test]
    fn nested_calls() {
        let input = json!({"doc": "{\"a\": [1, 2]}", "count": 4});
        assert_eq!(
            run(
                "States.JsonToString(States.StringToJson($.doc))",
                input.clone()
            )
            .unwrap(),
            json!(r#"{"a":[1,2]}"#)
        );
        assert_eq!(
            run(
                "States.MathAdd(States.MathAdd($.count, 1), -10)",
                input.clone()
            )
            .unwrap(),
            json!(-5)
        );
        assert_eq!(
            run("States.Format('n={}', States.MathAdd($.count, 1))", input).unwrap(),
            json!("n=5")
        );
    }

    #[test]
    fn path_arguments_with_brackets() {
        let input = json!({"a b": {"c": [7, 8]}});
        assert_eq!(
            run("States.MathAdd($['a b'].c[1], 0)", input).unwrap(),
            json!(8)
        );
    }

    #[test]
    fn errors() {
        assert!(matches!(
            run("States.MathAdd(1.5, 1)", json!({})),
            Err(IntrinsicError::Call { .. })
        ));
        assert!(matches!(
            run("States.MathAdd($.x, 1)", json!({})),
            Err(IntrinsicError::Path(_))
        ));
        assert!(matches!(
            run("States.Nope(1)", json!({})),
            Err(IntrinsicError::Call { .. })
        ));
        assert!(matches!(
            run("States.StringToJson('{')", json!({})),
            Err(IntrinsicError::Call { .. })
        ));
        assert!(matches!(
            parse("States.Format('a'"),
            Err(IntrinsicError::Parse { .. })
        ));
        assert!(matches!(
            parse("States.Format('a') x"),
            Err(IntrinsicError::Parse { .. })
        ));
        assert!(matches!(
            parse("Format('a')"),
            Err(IntrinsicError::Parse { .. })
        ));
        assert!(matches!(
            parse("States.Format('a\\n')"),
            Err(IntrinsicError::Parse { .. })
        ));
    }
}
