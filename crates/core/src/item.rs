use std::collections::HashMap;

use aws_sdk_dynamodb::types::AttributeValue;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::store::DbError;

pub type Item = HashMap<String, AttributeValue>;

pub const KIND: &str = "kind";
pub const VERSION: &str = "v";
pub const TTL: &str = "expires_at";
pub const BLOB: &str = "$blob";
const TIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.6fZ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Table {
    Main,
    Audit,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub table: Table,
    pub pk: String,
    pub sk: String,
}

impl Key {
    pub fn main(pk: impl Into<String>, sk: impl Into<String>) -> Self {
        Self {
            table: Table::Main,
            pk: pk.into(),
            sk: sk.into(),
        }
    }

    pub fn audit(pk: impl Into<String>, sk: impl Into<String>) -> Self {
        Self {
            table: Table::Audit,
            pk: pk.into(),
            sk: sk.into(),
        }
    }

    pub fn attributes(&self) -> Item {
        HashMap::from([
            ("pk".to_string(), AttributeValue::S(self.pk.clone())),
            ("sk".to_string(), AttributeValue::S(self.sk.clone())),
        ])
    }

    pub fn of(table: Table, item: &Item) -> Result<Self, DbError> {
        let read = ItemReader::new(item);
        Ok(Self {
            table,
            pk: read.s("pk")?,
            sk: read.s("sk")?,
        })
    }
}

pub fn format_time(time: DateTime<Utc>) -> String {
    time.format(TIME_FORMAT).to_string()
}

pub fn parse_time(text: &str) -> Result<DateTime<Utc>, DbError> {
    NaiveDateTime::parse_from_str(text, TIME_FORMAT)
        .map(|naive| naive.and_utc())
        .map_err(|e| DbError::Decode(format!("time {text:?}: {e}")))
}

pub fn padded(number: i64) -> String {
    format!("{number:012}")
}

pub trait Entity: Sized {
    const KIND: &'static str;
    fn key(&self) -> Key;
    fn write(&self, item: &mut ItemWriter);
    fn read(item: &ItemReader<'_>) -> Result<Self, DbError>;

    fn to_item(&self) -> Item {
        let key = self.key();
        let mut writer = ItemWriter::new(Self::KIND);
        writer.s("pk", &key.pk).s("sk", &key.sk);
        self.write(&mut writer);
        writer.finish()
    }

    fn from_item(item: &Item) -> Result<Self, DbError> {
        Self::read(&ItemReader::new(item))
    }
}

#[derive(Debug, Default, Clone)]
pub struct ItemWriter {
    item: Item,
}

impl ItemWriter {
    pub fn new(kind: &str) -> Self {
        let mut item = HashMap::new();
        item.insert(KIND.to_string(), AttributeValue::S(kind.to_string()));
        Self { item }
    }

    pub fn finish(self) -> Item {
        self.item
    }

    pub fn raw(&mut self, name: &str, value: AttributeValue) -> &mut Self {
        self.item.insert(name.to_string(), value);
        self
    }

    pub fn s(&mut self, name: &str, value: impl AsRef<str>) -> &mut Self {
        self.raw(name, AttributeValue::S(value.as_ref().to_string()))
    }

    pub fn opt_s(&mut self, name: &str, value: Option<impl AsRef<str>>) -> &mut Self {
        if let Some(value) = value {
            self.s(name, value);
        }
        self
    }

    pub fn null_or_n(&mut self, name: &str, value: Option<i64>) -> &mut Self {
        match value {
            Some(value) => self.n(name, value),
            None => self.raw(name, AttributeValue::Null(true)),
        }
    }

    pub fn n(&mut self, name: &str, value: i64) -> &mut Self {
        self.raw(name, AttributeValue::N(value.to_string()))
    }

    pub fn opt_n(&mut self, name: &str, value: Option<i64>) -> &mut Self {
        if let Some(value) = value {
            self.n(name, value);
        }
        self
    }

    pub fn f(&mut self, name: &str, value: f64) -> &mut Self {
        self.raw(name, AttributeValue::N(value.to_string()))
    }

    pub fn b(&mut self, name: &str, value: bool) -> &mut Self {
        self.raw(name, AttributeValue::Bool(value))
    }

    pub fn uuid(&mut self, name: &str, value: Uuid) -> &mut Self {
        self.s(name, value.to_string())
    }

    pub fn opt_uuid(&mut self, name: &str, value: Option<Uuid>) -> &mut Self {
        self.opt_s(name, value.map(|v| v.to_string()))
    }

    pub fn time(&mut self, name: &str, value: DateTime<Utc>) -> &mut Self {
        self.s(name, format_time(value))
    }

    pub fn opt_time(&mut self, name: &str, value: Option<DateTime<Utc>>) -> &mut Self {
        self.opt_s(name, value.map(format_time))
    }

    pub fn ttl(&mut self, at: DateTime<Utc>) -> &mut Self {
        self.n(TTL, at.timestamp())
    }

    pub fn json(&mut self, name: &str, value: &Value) -> &mut Self {
        self.s(
            name,
            serde_json::to_string(value).expect("JSON values serialize"),
        )
    }

    pub fn opt_json(&mut self, name: &str, value: Option<&Value>) -> &mut Self {
        if let Some(value) = value {
            self.json(name, value);
        }
        self
    }

    pub fn strs(&mut self, name: &str, values: &[String]) -> &mut Self {
        self.raw(
            name,
            AttributeValue::L(values.iter().cloned().map(AttributeValue::S).collect()),
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ItemReader<'a> {
    item: &'a Item,
}

impl<'a> ItemReader<'a> {
    pub fn new(item: &'a Item) -> Self {
        Self { item }
    }

    fn missing(&self, name: &str) -> DbError {
        let kind = self
            .item
            .get(KIND)
            .and_then(|v| v.as_s().ok())
            .map(String::as_str)
            .unwrap_or("item");
        DbError::Decode(format!("{kind} has no attribute {name}"))
    }

    fn wrong(&self, name: &str, expected: &str) -> DbError {
        DbError::Decode(format!("attribute {name} is not {expected}"))
    }

    pub fn has(&self, name: &str) -> bool {
        self.item.contains_key(name)
    }

    pub fn opt_s(&self, name: &str) -> Result<Option<String>, DbError> {
        match self.item.get(name) {
            None | Some(AttributeValue::Null(_)) => Ok(None),
            Some(AttributeValue::S(s)) => Ok(Some(s.clone())),
            Some(_) => Err(self.wrong(name, "a string")),
        }
    }

    pub fn s(&self, name: &str) -> Result<String, DbError> {
        self.opt_s(name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_n(&self, name: &str) -> Result<Option<i64>, DbError> {
        match self.item.get(name) {
            None | Some(AttributeValue::Null(_)) => Ok(None),
            Some(AttributeValue::N(n)) => n
                .parse()
                .map(Some)
                .map_err(|_| self.wrong(name, "an integer")),
            Some(_) => Err(self.wrong(name, "a number")),
        }
    }

    pub fn null_or_n(&self, name: &str) -> Result<Option<i64>, DbError> {
        if !self.has(name) {
            return Err(self.missing(name));
        }
        self.opt_n(name)
    }

    pub fn n(&self, name: &str) -> Result<i64, DbError> {
        self.opt_n(name)?.ok_or_else(|| self.missing(name))
    }

    pub fn i32(&self, name: &str) -> Result<i32, DbError> {
        i32::try_from(self.n(name)?).map_err(|_| self.wrong(name, "an i32"))
    }

    pub fn f(&self, name: &str) -> Result<f64, DbError> {
        match self.item.get(name) {
            Some(AttributeValue::N(n)) => n.parse().map_err(|_| self.wrong(name, "a float")),
            Some(_) => Err(self.wrong(name, "a number")),
            None => Err(self.missing(name)),
        }
    }

    pub fn b(&self, name: &str) -> Result<bool, DbError> {
        match self.item.get(name) {
            Some(AttributeValue::Bool(b)) => Ok(*b),
            Some(_) => Err(self.wrong(name, "a boolean")),
            None => Err(self.missing(name)),
        }
    }

    pub fn uuid(&self, name: &str) -> Result<Uuid, DbError> {
        self.s(name)?
            .parse()
            .map_err(|_| self.wrong(name, "a UUID"))
    }

    pub fn opt_uuid(&self, name: &str) -> Result<Option<Uuid>, DbError> {
        self.opt_s(name)?
            .map(|s| s.parse().map_err(|_| self.wrong(name, "a UUID")))
            .transpose()
    }

    pub fn time(&self, name: &str) -> Result<DateTime<Utc>, DbError> {
        parse_time(&self.s(name)?)
    }

    pub fn opt_time(&self, name: &str) -> Result<Option<DateTime<Utc>>, DbError> {
        self.opt_s(name)?.map(|s| parse_time(&s)).transpose()
    }

    pub fn json(&self, name: &str) -> Result<Value, DbError> {
        serde_json::from_str(&self.s(name)?)
            .map_err(|e| DbError::Decode(format!("attribute {name}: {e}")))
    }

    pub fn opt_json(&self, name: &str) -> Result<Option<Value>, DbError> {
        self.opt_s(name)?
            .map(|s| {
                serde_json::from_str(&s)
                    .map_err(|e| DbError::Decode(format!("attribute {name}: {e}")))
            })
            .transpose()
    }

    pub fn strs(&self, name: &str) -> Result<Vec<String>, DbError> {
        match self.item.get(name) {
            None => Ok(Vec::new()),
            Some(AttributeValue::L(items)) => items
                .iter()
                .map(|v| {
                    v.as_s()
                        .cloned()
                        .map_err(|_| self.wrong(name, "a list of strings"))
                })
                .collect(),
            Some(_) => Err(self.wrong(name, "a list")),
        }
    }

    pub fn parse<T: std::str::FromStr>(&self, name: &str) -> Result<T, DbError> {
        self.s(name)?
            .parse()
            .map_err(|_| self.wrong(name, std::any::type_name::<T>()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn times_sort_lexicographically_and_round_trip() {
        let a = Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap();
        let b = a + chrono::Duration::microseconds(1);
        assert!(format_time(a) < format_time(b));
        assert_eq!(parse_time(&format_time(b)).unwrap(), b);
        assert!(padded(9) < padded(10));
    }

    #[test]
    fn readers_report_missing_and_mistyped_attributes() {
        let mut writer = ItemWriter::new("task");
        writer
            .s("name", "x")
            .n("turns", 3)
            .json("payload", &serde_json::json!({"a": 1}));
        let item = writer.finish();
        let read = ItemReader::new(&item);
        assert_eq!(read.n("turns").unwrap(), 3);
        assert_eq!(read.json("payload").unwrap()["a"], 1);
        assert!(read.opt_s("missing").unwrap().is_none());
        assert!(
            matches!(read.s("missing"), Err(DbError::Decode(m)) if m.contains("task has no attribute missing"))
        );
        assert!(read.n("name").is_err());
    }
}
