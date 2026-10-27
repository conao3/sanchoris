use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRoot {
    Input,
    Context,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Field(String),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencePath {
    pub root: PathRoot,
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PathError {
    #[error("invalid path {path:?}: {reason}")]
    Invalid { path: String, reason: String },
    #[error("path {0:?} does not match any value")]
    NotFound(String),
    #[error("cannot write to path {path:?}: {reason}")]
    Write { path: String, reason: String },
}

impl ReferencePath {
    pub fn parse(text: &str) -> Result<Self, PathError> {
        let invalid = |reason: &str| PathError::Invalid {
            path: text.to_string(),
            reason: reason.to_string(),
        };
        let (root, rest) = if let Some(rest) = text.strip_prefix("$$") {
            (PathRoot::Context, rest)
        } else if let Some(rest) = text.strip_prefix('$') {
            (PathRoot::Input, rest)
        } else {
            return Err(invalid("a path starts with $ or $$"));
        };
        let chars: Vec<char> = rest.chars().collect();
        let mut segments = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '.' => {
                    i += 1;
                    let start = i;
                    while i < chars.len() && chars[i] != '.' && chars[i] != '[' {
                        i += 1;
                    }
                    let name: String = chars[start..i].iter().collect();
                    if name.is_empty() {
                        return Err(invalid("empty field name"));
                    }
                    if name.contains(['*', '?', '@', ']', '(', ')', ' ', ',', '\'', '"']) {
                        return Err(invalid("only reference paths are supported"));
                    }
                    segments.push(Segment::Field(name));
                }
                '[' => {
                    i += 1;
                    let close = chars[i..]
                        .iter()
                        .position(|c| *c == ']')
                        .map(|p| p + i)
                        .ok_or_else(|| invalid("unclosed ["))?;
                    let inner: String = chars[i..close].iter().collect();
                    let segment = if let Some(quoted) = inner
                        .strip_prefix('\'')
                        .and_then(|s| s.strip_suffix('\''))
                        .or_else(|| inner.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
                    {
                        if quoted.is_empty() {
                            return Err(invalid("empty field name"));
                        }
                        Segment::Field(quoted.to_string())
                    } else {
                        let index = inner
                            .parse::<usize>()
                            .map_err(|_| invalid("only reference paths are supported"))?;
                        Segment::Index(index)
                    };
                    segments.push(segment);
                    i = close + 1;
                }
                _ => return Err(invalid("expected . or [")),
            }
        }
        Ok(ReferencePath { root, segments })
    }

    pub fn select<'v>(&self, input: &'v Value, context: &'v Value) -> Option<&'v Value> {
        let base = match self.root {
            PathRoot::Input => input,
            PathRoot::Context => context,
        };
        self.segments
            .iter()
            .try_fold(base, |value, segment| match segment {
                Segment::Field(name) => value.as_object()?.get(name),
                Segment::Index(index) => value.as_array()?.get(*index),
            })
    }

    pub fn write(&self, document: &Value, value: Value) -> Result<Value, PathError> {
        if self.root == PathRoot::Context {
            return Err(PathError::Write {
                path: self.to_string(),
                reason: "the context object is read-only".to_string(),
            });
        }
        let mut out = document.clone();
        write_into(&mut out, &self.segments, value).map_err(|reason| PathError::Write {
            path: self.to_string(),
            reason,
        })?;
        Ok(out)
    }
}

fn write_into(target: &mut Value, segments: &[Segment], value: Value) -> Result<(), String> {
    let Some((first, rest)) = segments.split_first() else {
        *target = value;
        return Ok(());
    };
    match first {
        Segment::Field(name) => {
            let object = target
                .as_object_mut()
                .ok_or_else(|| format!("{name:?} is not inside an object"))?;
            let slot = object
                .entry(name.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            write_into(slot, rest, value)
        }
        Segment::Index(index) => {
            let array = target
                .as_array_mut()
                .ok_or_else(|| format!("[{index}] is not inside an array"))?;
            let len = array.len();
            let slot = array
                .get_mut(*index)
                .ok_or_else(|| format!("[{index}] is out of bounds for length {len}"))?;
            write_into(slot, rest, value)
        }
    }
}

impl std::fmt::Display for ReferencePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.root {
            PathRoot::Input => "$",
            PathRoot::Context => "$$",
        })?;
        for segment in &self.segments {
            match segment {
                Segment::Field(name) => write!(f, ".{name}")?,
                Segment::Index(index) => write!(f, "[{index}]")?,
            }
        }
        Ok(())
    }
}

pub fn select_required<'v>(
    path: &str,
    input: &'v Value,
    context: &'v Value,
) -> Result<&'v Value, PathError> {
    ReferencePath::parse(path)?
        .select(input, context)
        .ok_or_else(|| PathError::NotFound(path.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_dot_and_bracket_segments() {
        let path = ReferencePath::parse("$.a['b c'][2].d").unwrap();
        assert_eq!(path.root, PathRoot::Input);
        assert_eq!(
            path.segments,
            vec![
                Segment::Field("a".into()),
                Segment::Field("b c".into()),
                Segment::Index(2),
                Segment::Field("d".into()),
            ]
        );
        assert_eq!(
            ReferencePath::parse("$$.Task.Token").unwrap().root,
            PathRoot::Context
        );
        assert!(ReferencePath::parse("$").unwrap().segments.is_empty());
    }

    #[test]
    fn rejects_non_reference_paths() {
        for bad in ["a.b", "$.", "$..a", "$.a[*]", "$.a[?(@.b)]", "$.a[", "$a"] {
            assert!(
                ReferencePath::parse(bad).is_err(),
                "{bad} should be rejected"
            );
        }
    }

    #[test]
    fn selects_values() {
        let input = json!({"a": {"b": [10, 20]}, "n": null});
        let context = json!({"State": {"Name": "Push"}});
        let get = |p: &str| {
            ReferencePath::parse(p)
                .unwrap()
                .select(&input, &context)
                .cloned()
        };
        assert_eq!(get("$.a.b[1]"), Some(json!(20)));
        assert_eq!(get("$.n"), Some(Value::Null));
        assert_eq!(get("$.a.c"), None);
        assert_eq!(get("$.a.b[5]"), None);
        assert_eq!(get("$$.State.Name"), Some(json!("Push")));
        assert_eq!(get("$"), Some(input.clone()));
    }

    #[test]
    fn writes_create_intermediate_objects() {
        let doc = json!({"keep": 1});
        let out = ReferencePath::parse("$.x.y.z")
            .unwrap()
            .write(&doc, json!(true))
            .unwrap();
        assert_eq!(out, json!({"keep": 1, "x": {"y": {"z": true}}}));
        let out = ReferencePath::parse("$.keep")
            .unwrap()
            .write(&doc, json!(2))
            .unwrap();
        assert_eq!(out, json!({"keep": 2}));
        let out = ReferencePath::parse("$")
            .unwrap()
            .write(&doc, json!("r"))
            .unwrap();
        assert_eq!(out, json!("r"));
    }

    #[test]
    fn write_fails_through_non_objects() {
        let doc = json!({"keep": 1, "list": [1]});
        assert!(
            ReferencePath::parse("$.keep.x")
                .unwrap()
                .write(&doc, json!(0))
                .is_err()
        );
        assert!(
            ReferencePath::parse("$.list[3]")
                .unwrap()
                .write(&doc, json!(0))
                .is_err()
        );
        assert!(
            ReferencePath::parse("$.a")
                .unwrap()
                .write(&json!("s"), json!(0))
                .is_err()
        );
        assert!(
            ReferencePath::parse("$$.a")
                .unwrap()
                .write(&doc, json!(0))
                .is_err()
        );
    }
}
