use crate::error::{ValidationError, at, index};
use indexmap::IndexMap;
use serde_json::{Map, Value};

pub(crate) struct Decoder<'e> {
    pub errors: &'e mut Vec<ValidationError>,
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "a mapping",
    }
}

impl Decoder<'_> {
    pub fn error(&mut self, location: impl Into<String>, message: impl Into<String>) {
        self.errors.push(ValidationError::new(location, message));
    }

    pub fn object<'v>(
        &mut self,
        v: &'v Value,
        loc: &str,
        allowed: &[&str],
    ) -> Option<&'v Map<String, Value>> {
        let Value::Object(m) = v else {
            self.error(loc, format!("expected a mapping, found {}", kind(v)));
            return None;
        };
        for key in m.keys() {
            if !allowed.contains(&key.as_str()) {
                self.error(
                    at(loc, key),
                    format!("unknown field; expected one of {}", allowed.join(", ")),
                );
            }
        }
        Some(m)
    }

    pub fn field<T>(
        &mut self,
        m: &Map<String, Value>,
        loc: &str,
        key: &str,
        f: impl FnOnce(&mut Self, &Value, &str) -> Option<T>,
    ) -> Option<T> {
        let path = at(loc, key);
        match m.get(key) {
            Some(v) => f(self, v, &path),
            None => {
                self.error(path, "missing required field");
                None
            }
        }
    }

    pub fn optional<T>(
        &mut self,
        m: &Map<String, Value>,
        loc: &str,
        key: &str,
        f: impl FnOnce(&mut Self, &Value, &str) -> Option<T>,
    ) -> Option<Option<T>> {
        match m.get(key) {
            Some(v) => f(self, v, &at(loc, key)).map(Some),
            None => Some(None),
        }
    }

    pub fn string(&mut self, v: &Value, loc: &str) -> Option<String> {
        match v {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::String(_) => {
                self.error(loc, "must not be empty");
                None
            }
            _ => {
                self.error(loc, format!("expected a string, found {}", kind(v)));
                None
            }
        }
    }

    pub fn text(&mut self, v: &Value, loc: &str) -> Option<String> {
        match v {
            Value::String(s) => Some(s.clone()),
            _ => {
                self.error(loc, format!("expected a string, found {}", kind(v)));
                None
            }
        }
    }

    pub fn u64(&mut self, v: &Value, loc: &str) -> Option<u64> {
        match v.as_u64() {
            Some(n) => Some(n),
            None => {
                self.error(
                    loc,
                    format!("expected a non-negative integer, found {}", kind(v)),
                );
                None
            }
        }
    }

    pub fn positive(&mut self, v: &Value, loc: &str) -> Option<u64> {
        let n = self.u64(v, loc)?;
        if n == 0 {
            self.error(loc, "must be greater than 0");
            return None;
        }
        Some(n)
    }

    pub fn f64(&mut self, v: &Value, loc: &str) -> Option<f64> {
        match v.as_f64() {
            Some(n) if n >= 0.0 => Some(n),
            _ => {
                self.error(
                    loc,
                    format!("expected a non-negative number, found {}", kind(v)),
                );
                None
            }
        }
    }

    pub fn enumeration<T: Copy>(
        &mut self,
        v: &Value,
        loc: &str,
        options: &[(&str, T)],
    ) -> Option<T> {
        let names: Vec<&str> = options.iter().map(|(n, _)| *n).collect();
        match v {
            Value::String(s) => match options.iter().find(|(n, _)| n == s) {
                Some((_, t)) => Some(*t),
                None => {
                    self.error(
                        loc,
                        format!("unknown value {s:?}; expected one of {}", names.join(", ")),
                    );
                    None
                }
            },
            _ => {
                self.error(loc, format!("expected one of {}", names.join(", ")));
                None
            }
        }
    }

    pub fn list<T>(
        &mut self,
        v: &Value,
        loc: &str,
        mut f: impl FnMut(&mut Self, &Value, &str) -> Option<T>,
    ) -> Option<Vec<T>> {
        let Value::Array(items) = v else {
            self.error(loc, format!("expected a list, found {}", kind(v)));
            return None;
        };
        let mut out = Vec::with_capacity(items.len());
        let mut ok = true;
        for (i, item) in items.iter().enumerate() {
            match f(self, item, &index(loc, i)) {
                Some(t) => out.push(t),
                None => ok = false,
            }
        }
        ok.then_some(out)
    }

    pub fn strings(&mut self, v: &Value, loc: &str) -> Option<Vec<String>> {
        self.list(v, loc, |d, v, loc| d.string(v, loc))
    }

    pub fn map<T>(
        &mut self,
        v: &Value,
        loc: &str,
        mut f: impl FnMut(&mut Self, &Value, &str) -> Option<T>,
    ) -> Option<IndexMap<String, T>> {
        let Value::Object(m) = v else {
            self.error(loc, format!("expected a mapping, found {}", kind(v)));
            return None;
        };
        let mut out = IndexMap::with_capacity(m.len());
        let mut ok = true;
        for (key, item) in m {
            match f(self, item, &at(loc, key)) {
                Some(t) => {
                    out.insert(key.clone(), t);
                }
                None => ok = false,
            }
        }
        ok.then_some(out)
    }
}
