use async_graphql::{InputValueError, InputValueResult, Scalar, ScalarType, Value};
use chrono::{DateTime, SecondsFormat, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(pub DateTime<Utc>);

#[Scalar(name = "DateTime")]
impl ScalarType for Timestamp {
    fn parse(value: Value) -> InputValueResult<Self> {
        match &value {
            Value::String(text) => DateTime::parse_from_rfc3339(text)
                .map(|at| Timestamp(at.with_timezone(&Utc)))
                .map_err(|e| InputValueError::custom(format!("{text} is not ISO 8601: {e}"))),
            _ => Err(InputValueError::expected_type(value)),
        }
    }

    fn to_value(&self) -> Value {
        Value::String(self.0.to_rfc3339_opts(SecondsFormat::Secs, true))
    }
}

impl From<DateTime<Utc>> for Timestamp {
    fn from(at: DateTime<Utc>) -> Self {
        Self(at)
    }
}

pub fn ts(at: DateTime<Utc>) -> Timestamp {
    Timestamp(at)
}

pub fn ts_opt(at: Option<DateTime<Utc>>) -> Option<Timestamp> {
    at.map(Timestamp)
}
