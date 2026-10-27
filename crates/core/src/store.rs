use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use aws_sdk_dynamodb::error::DisplayErrorContext;
use aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsError;
use aws_sdk_dynamodb::types::{
    AttributeValue, ConditionCheck, Delete, Put, TransactWriteItem, Update as UpdateOp,
};
use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::aws::{MissingEnv, require_env};
use crate::clock::Clock;
use crate::item::{BLOB, Entity, Item, ItemReader, ItemWriter, KIND, Key, TTL, Table, VERSION};
use crate::jobs::{Envelope, Job, Relayed, schedule_name};
use crate::ports::{BlobStore, OutgoingMessage, PortError, Queue, Scheduler};

pub const MAIN_TABLE_ENV: &str = "SANCHORIS_TABLE_MAIN";
pub const AUDIT_TABLE_ENV: &str = "SANCHORIS_TABLE_AUDIT";
pub const MAX_TRANSACTION_ITEMS: usize = 100;
pub const DOC_INLINE_LIMIT: usize = 32 * 1024;
pub const MESSAGE_RETENTION_HOURS: i64 = 24;
pub const GLOBAL_PREFIXES: [&str; 4] = ["UNIQ#", "USER#", "INVITEMAIL#", "MSG#"];

#[derive(Debug, Error)]
pub enum DbError {
    #[error("DynamoDB request failed: {0}")]
    Dynamo(String),
    #[error("the transaction conflicted with a concurrent write")]
    Conflict,
    #[error("a transaction can write at most {MAX_TRANSACTION_ITEMS} items, not {0}")]
    TooManyWrites(usize),
    #[error("stored item does not decode: {0}")]
    Decode(String),
    #[error("failed to serialize: {0}")]
    Definition(#[from] serde_json::Error),
    #[error(transparent)]
    Port(#[from] PortError),
    #[error("tenant {tenant} cannot access key {pk}")]
    CrossTenant { tenant: Uuid, pk: String },
    #[error("{0} must be read in this transaction before it is replaced")]
    Unread(String),
    #[error("{0} does not exist")]
    Missing(String),
    #[error("audit event {0} is append-only")]
    AppendOnly(String),
    #[error("trigger event {0} was already evaluated or does not exist")]
    TriggerEventNotPending(Uuid),
    #[error("tenant {0} does not exist")]
    TenantNotFound(Uuid),
    #[error("{0} conflicted but the existing row is not visible")]
    ConflictNotVisible(&'static str),
    #[error("no free tenant slug starts with {0}")]
    SlugExhausted(String),
}

impl DbError {
    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Conflict)
    }
}

fn dynamo_error(error: impl std::error::Error) -> DbError {
    DbError::Dynamo(DisplayErrorContext(error).to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tables {
    pub main: String,
    pub audit: String,
}

impl Tables {
    pub fn from_env() -> Result<Self, MissingEnv> {
        let [main, audit] = require_env([MAIN_TABLE_ENV, AUDIT_TABLE_ENV])?;
        Ok(Self { main, audit })
    }

    pub fn name(&self, table: Table) -> &str {
        match table {
            Table::Main => &self.main,
            Table::Audit => &self.audit,
        }
    }
}

#[derive(Clone)]
pub enum Relay {
    Stream,
    Direct {
        queue: Arc<dyn Queue>,
        scheduler: Arc<dyn Scheduler>,
        clock: Arc<dyn Clock>,
    },
}

pub async fn relay(
    queue: &dyn Queue,
    scheduler: &dyn Scheduler,
    now: DateTime<Utc>,
    relayed: &Relayed,
) -> Result<(), PortError> {
    match relayed {
        Relayed::Send {
            envelope,
            deliver_at,
        } => {
            let message = OutgoingMessage {
                body: envelope.body(),
                group: envelope.job.group(),
                dedupe_id: envelope.id.to_string(),
                delay: None,
            };
            match deliver_at {
                Some(at) if *at > now => {
                    scheduler
                        .schedule(&schedule_name(envelope.id), *at, &message)
                        .await
                }
                _ => queue.send(&message).await,
            }
        }
        Relayed::Cancel { schedule } => scheduler.cancel(schedule).await,
    }
}

pub fn relayed_from_item(item: &Item) -> Result<Option<Relayed>, DbError> {
    let read = ItemReader::new(item);
    if read.opt_s(KIND)?.as_deref() != Some(MESSAGE_KIND) {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&read.s("body")?).map_err(
        |e| DbError::Decode(format!("message body: {e}")),
    )?))
}

const MESSAGE_KIND: &str = "msg";

struct Inner {
    client: aws_sdk_dynamodb::Client,
    tables: Tables,
    blobs: Arc<dyn BlobStore>,
    relay: Relay,
}

#[derive(Clone)]
pub struct Store {
    inner: Arc<Inner>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Index {
    Gsi1,
    Gsi2,
}

impl Index {
    fn name(self) -> &'static str {
        match self {
            Self::Gsi1 => "gsi1",
            Self::Gsi2 => "gsi2",
        }
    }

    fn attributes(self) -> (&'static str, &'static str) {
        match self {
            Self::Gsi1 => ("gsi1pk", "gsi1sk"),
            Self::Gsi2 => ("gsi2pk", "gsi2sk"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SortKey {
    All,
    Prefix(String),
    AtMost(String),
    AtLeast(String),
    Between(String, String),
}

impl SortKey {
    pub fn prefix(prefix: &str) -> Self {
        if prefix.is_empty() {
            Self::All
        } else {
            Self::Prefix(prefix.to_string())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub table: Table,
    pub index: Option<Index>,
    pub pk: String,
    pub sort: SortKey,
    pub forward: bool,
    pub limit: Option<usize>,
}

impl Listing {
    pub fn main(pk: impl Into<String>) -> Self {
        Self {
            table: Table::Main,
            index: None,
            pk: pk.into(),
            sort: SortKey::All,
            forward: true,
            limit: None,
        }
    }

    pub fn index(index: Index, pk: impl Into<String>) -> Self {
        Self {
            index: Some(index),
            ..Self::main(pk)
        }
    }

    pub fn audit(pk: impl Into<String>) -> Self {
        Self {
            table: Table::Audit,
            ..Self::main(pk)
        }
    }

    pub fn sort(mut self, sort: SortKey) -> Self {
        self.sort = sort;
        self
    }

    pub fn prefix(self, prefix: &str) -> Self {
        self.sort(SortKey::prefix(prefix))
    }

    pub fn descending(mut self) -> Self {
        self.forward = false;
        self
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
}

const LIST_PAGE: usize = 50;

impl Store {
    pub fn new(
        client: aws_sdk_dynamodb::Client,
        tables: Tables,
        blobs: Arc<dyn BlobStore>,
        relay: Relay,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                client,
                tables,
                blobs,
                relay,
            }),
        }
    }

    pub fn from_env(
        config: &aws_config::SdkConfig,
        blobs: Arc<dyn BlobStore>,
        relay: Relay,
    ) -> Result<Self, MissingEnv> {
        Ok(Self::new(
            aws_sdk_dynamodb::Client::new(config),
            Tables::from_env()?,
            blobs,
            relay,
        ))
    }

    pub fn client(&self) -> &aws_sdk_dynamodb::Client {
        &self.inner.client
    }

    pub fn tables(&self) -> &Tables {
        &self.inner.tables
    }

    pub fn blobs(&self) -> &Arc<dyn BlobStore> {
        &self.inner.blobs
    }

    pub fn tx(&self, tenant_id: Uuid) -> TenantTx {
        TenantTx::begin(self, tenant_id)
    }

    pub fn global_tx(&self) -> TenantTx {
        TenantTx::begin(self, Uuid::nil())
    }

    async fn hydrate(&self, mut item: Item) -> Result<Item, DbError> {
        let blobs = item
            .iter()
            .filter_map(|(name, value)| match value {
                AttributeValue::M(map) if map.len() == 1 => map
                    .get(BLOB)
                    .and_then(|v| v.as_s().ok())
                    .map(|key| (name.clone(), key.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        for (name, key) in blobs {
            let body = self
                .inner
                .blobs
                .get(&key)
                .await?
                .ok_or_else(|| DbError::Missing(format!("blob {key}")))?;
            let text = String::from_utf8(body)
                .map_err(|_| DbError::Decode(format!("blob {key} is not UTF-8")))?;
            item.insert(name, AttributeValue::S(text));
        }
        Ok(item)
    }

    async fn dehydrate(&self, pk: &str, mut item: Item) -> Result<Item, DbError> {
        let owner = pk
            .strip_prefix("T#")
            .and_then(|rest| rest.split('#').next())
            .unwrap_or("global")
            .to_string();
        let large = item
            .iter()
            .filter_map(|(name, value)| match value {
                AttributeValue::S(text) if text.len() > DOC_INLINE_LIMIT => {
                    Some((name.clone(), text.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for (name, text) in large {
            let digest = Sha256::digest(text.as_bytes());
            let hash = digest
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let key = format!("docs/{owner}/{hash}");
            self.inner.blobs.put(&key, text.into_bytes()).await?;
            item.insert(
                name,
                AttributeValue::M(HashMap::from([(BLOB.to_string(), AttributeValue::S(key))])),
            );
        }
        Ok(item)
    }

    pub async fn get_item(&self, key: &Key) -> Result<Option<Item>, DbError> {
        let output = Box::pin(
            self.inner
                .client
                .get_item()
                .table_name(self.inner.tables.name(key.table))
                .set_key(Some(key.attributes()))
                .consistent_read(true)
                .send(),
        )
        .await
        .map_err(dynamo_error)?;
        match output.item {
            Some(item) => Ok(Some(self.hydrate(item).await?)),
            None => Ok(None),
        }
    }

    pub async fn get<E: Entity>(&self, key: &Key) -> Result<Option<E>, DbError> {
        self.get_item(key)
            .await?
            .map(|item| E::from_item(&item))
            .transpose()
    }

    pub async fn query_items(
        &self,
        table: Table,
        index: Option<Index>,
        pk: &str,
        sort: &SortKey,
        forward: bool,
        limit: Option<usize>,
    ) -> Result<Vec<Item>, DbError> {
        let listing = Listing {
            table,
            index,
            pk: pk.to_string(),
            sort: sort.clone(),
            forward,
            limit,
        };
        self.run_listing(&listing, false, |_| true).await
    }

    pub async fn list<E: Entity>(
        &self,
        listing: &Listing,
        keep: impl Fn(&E) -> bool,
    ) -> Result<Vec<E>, DbError> {
        let items = self
            .run_listing(listing, true, |item| {
                is_kind::<E>(item) && E::from_item(item).map_or(true, |entity| keep(&entity))
            })
            .await?;
        items.iter().map(E::from_item).collect()
    }

    async fn run_listing(
        &self,
        listing: &Listing,
        filtered: bool,
        keep: impl Fn(&Item) -> bool,
    ) -> Result<Vec<Item>, DbError> {
        let (pk_name, sk_name) = match listing.index {
            Some(index) => index.attributes(),
            None => ("pk", "sk"),
        };
        let mut names = HashMap::from([
            ("#pk".to_string(), pk_name.to_string()),
            ("#sk".to_string(), sk_name.to_string()),
        ]);
        let mut values =
            HashMap::from([(":pk".to_string(), AttributeValue::S(listing.pk.clone()))]);
        let condition = match &listing.sort {
            SortKey::All => {
                names.remove("#sk");
                "#pk = :pk".to_string()
            }
            SortKey::Prefix(prefix) => {
                values.insert(":a".into(), AttributeValue::S(prefix.clone()));
                "#pk = :pk AND begins_with(#sk, :a)".to_string()
            }
            SortKey::AtMost(upper) => {
                values.insert(":a".into(), AttributeValue::S(upper.clone()));
                "#pk = :pk AND #sk <= :a".to_string()
            }
            SortKey::AtLeast(lower) => {
                values.insert(":a".into(), AttributeValue::S(lower.clone()));
                "#pk = :pk AND #sk >= :a".to_string()
            }
            SortKey::Between(lower, upper) => {
                values.insert(":a".into(), AttributeValue::S(lower.clone()));
                values.insert(":b".into(), AttributeValue::S(upper.clone()));
                "#pk = :pk AND #sk BETWEEN :a AND :b".to_string()
            }
        };
        let full = |items: &Vec<Item>| listing.limit.is_some_and(|limit| items.len() >= limit);
        let mut items = Vec::new();
        let mut start = None;
        loop {
            let mut request = self
                .inner
                .client
                .query()
                .table_name(self.inner.tables.name(listing.table))
                .key_condition_expression(&condition)
                .set_expression_attribute_names(Some(names.clone()))
                .set_expression_attribute_values(Some(values.clone()))
                .scan_index_forward(listing.forward)
                .set_exclusive_start_key(start.take());
            match listing.index {
                Some(index) => request = request.index_name(index.name()),
                None => request = request.consistent_read(true),
            }
            if let Some(limit) = listing.limit {
                let remaining = limit.saturating_sub(items.len());
                let page = if filtered {
                    remaining.max(LIST_PAGE)
                } else {
                    remaining
                };
                request = request.limit(i32::try_from(page).unwrap_or(i32::MAX));
            }
            let output = Box::pin(request.send()).await.map_err(dynamo_error)?;
            for item in output.items.unwrap_or_default() {
                if full(&items) {
                    break;
                }
                let item = self.hydrate(item).await?;
                if keep(&item) {
                    items.push(item);
                }
            }
            if full(&items) {
                break;
            }
            match output.last_evaluated_key {
                Some(key) if !key.is_empty() => start = Some(key),
                _ => break,
            }
        }
        Ok(items)
    }

    pub async fn query<E: Entity>(&self, pk: &str, prefix: &str) -> Result<Vec<E>, DbError> {
        decode_kind(
            self.query_items(Table::Main, None, pk, &SortKey::prefix(prefix), true, None)
                .await?,
        )
    }

    pub async fn query_index<E: Entity>(
        &self,
        index: Index,
        pk: &str,
        sort: SortKey,
    ) -> Result<Vec<E>, DbError> {
        decode_kind(
            self.query_items(Table::Main, Some(index), pk, &sort, true, None)
                .await?,
        )
    }

    async fn transact(&self, writes: Vec<TransactWriteItem>) -> Result<(), DbError> {
        if writes.len() > MAX_TRANSACTION_ITEMS {
            return Err(DbError::TooManyWrites(writes.len()));
        }
        let result = Box::pin(
            self.inner
                .client
                .transact_write_items()
                .set_transact_items(Some(writes))
                .send(),
        )
        .await;
        match result {
            Ok(_) => Ok(()),
            Err(error) => match error.as_service_error() {
                Some(TransactWriteItemsError::TransactionCanceledException(cancelled)) => {
                    let retryable = cancelled.cancellation_reasons().iter().any(|reason| {
                        matches!(
                            reason.code(),
                            Some("ConditionalCheckFailed")
                                | Some("TransactionConflict")
                                | Some("ThrottlingError")
                                | Some("ProvisionedThroughputExceeded")
                        )
                    });
                    if retryable {
                        Err(DbError::Conflict)
                    } else {
                        Err(dynamo_error(error))
                    }
                }
                Some(TransactWriteItemsError::TransactionInProgressException(_)) => {
                    Err(DbError::Conflict)
                }
                _ => Err(dynamo_error(error)),
            },
        }
    }

    pub async fn deliver(&self, relayed: &[Relayed]) -> Result<(), DbError> {
        match &self.inner.relay {
            Relay::Stream => Ok(()),
            Relay::Direct {
                queue,
                scheduler,
                clock,
            } => {
                for message in relayed {
                    relay(queue.as_ref(), scheduler.as_ref(), clock.now(), message).await?;
                }
                Ok(())
            }
        }
    }
}

fn is_kind<E: Entity>(item: &Item) -> bool {
    item.get(KIND)
        .and_then(|v| v.as_s().ok())
        .is_some_and(|kind| kind == E::KIND)
}

fn decode_kind<E: Entity>(items: Vec<Item>) -> Result<Vec<E>, DbError> {
    items
        .iter()
        .filter(|item| is_kind::<E>(item))
        .map(E::from_item)
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Update {
    adds: BTreeMap<String, i64>,
    float_adds: BTreeMap<String, f64>,
    sets: BTreeMap<String, AttributeValue>,
    removes: BTreeSet<String>,
    caps: BTreeMap<String, i64>,
    equals: BTreeMap<String, AttributeValue>,
}

impl Update {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(mut self, name: &str, delta: i64) -> Self {
        *self.adds.entry(name.to_string()).or_default() += delta;
        self
    }

    pub fn add_float(mut self, name: &str, delta: f64) -> Self {
        *self.float_adds.entry(name.to_string()).or_default() += delta;
        self
    }

    pub fn set(mut self, name: &str, value: AttributeValue) -> Self {
        self.removes.remove(name);
        self.sets.insert(name.to_string(), value);
        self
    }

    pub fn set_s(self, name: &str, value: impl Into<String>) -> Self {
        self.set(name, AttributeValue::S(value.into()))
    }

    pub fn set_n(self, name: &str, value: i64) -> Self {
        self.set(name, AttributeValue::N(value.to_string()))
    }

    pub fn remove(mut self, name: &str) -> Self {
        self.sets.remove(name);
        self.removes.insert(name.to_string());
        self
    }

    pub fn cap_after(mut self, name: &str, max: i64) -> Self {
        let entry = self.caps.entry(name.to_string()).or_insert(max);
        *entry = (*entry).min(max);
        self
    }

    pub fn require(mut self, name: &str, value: AttributeValue) -> Self {
        self.equals.insert(name.to_string(), value);
        self
    }

    fn merge(&mut self, other: Update) {
        for (name, delta) in other.adds {
            *self.adds.entry(name).or_default() += delta;
        }
        for (name, delta) in other.float_adds {
            *self.float_adds.entry(name).or_default() += delta;
        }
        for name in other.removes {
            self.sets.remove(&name);
            self.removes.insert(name);
        }
        for (name, value) in other.sets {
            self.removes.remove(&name);
            self.sets.insert(name, value);
        }
        for (name, max) in other.caps {
            let entry = self.caps.entry(name).or_insert(max);
            *entry = (*entry).min(max);
        }
        self.equals.extend(other.equals);
    }

    fn apply(&self, item: &mut Item) {
        for (name, delta) in &self.adds {
            let current = item
                .get(name)
                .and_then(|v| v.as_n().ok())
                .and_then(|n| n.parse::<i64>().ok())
                .unwrap_or(0);
            item.insert(
                name.clone(),
                AttributeValue::N((current + delta).to_string()),
            );
        }
        for (name, delta) in &self.float_adds {
            let current = item
                .get(name)
                .and_then(|v| v.as_n().ok())
                .and_then(|n| n.parse::<f64>().ok())
                .unwrap_or(0.0);
            item.insert(
                name.clone(),
                AttributeValue::N((current + delta).to_string()),
            );
        }
        for (name, value) in &self.sets {
            item.insert(name.clone(), value.clone());
        }
        for name in &self.removes {
            item.remove(name);
        }
    }

    pub fn pending_delta(&self, name: &str) -> i64 {
        self.adds.get(name).copied().unwrap_or(0)
    }

    fn build(&self, table: &str, key: &Key) -> Result<UpdateOp, DbError> {
        let mut names = HashMap::from([("#pk".to_string(), "pk".to_string())]);
        let mut values = HashMap::new();
        let mut sets = Vec::new();
        let mut adds = vec!["#v :one".to_string()];
        names.insert("#v".into(), VERSION.into());
        values.insert(":one".into(), AttributeValue::N("1".into()));
        let mut removes = Vec::new();
        let mut conditions = vec!["attribute_exists(#pk)".to_string()];
        let mut n = 0;
        let mut next = |names: &mut HashMap<String, String>, attr: &str| {
            n += 1;
            let alias = format!("#a{n}");
            names.insert(alias.clone(), attr.to_string());
            (alias, format!(":a{n}"))
        };
        for (attr, delta) in &self.adds {
            let (alias, value) = next(&mut names, attr);
            adds.push(format!("{alias} {value}"));
            values.insert(value, AttributeValue::N(delta.to_string()));
        }
        for (attr, delta) in &self.float_adds {
            let (alias, value) = next(&mut names, attr);
            adds.push(format!("{alias} {value}"));
            values.insert(value, AttributeValue::N(delta.to_string()));
        }
        for (attr, set) in &self.sets {
            let (alias, value) = next(&mut names, attr);
            sets.push(format!("{alias} = {value}"));
            values.insert(value, set.clone());
        }
        for attr in &self.removes {
            let (alias, _) = next(&mut names, attr);
            removes.push(alias);
        }
        for (attr, max) in &self.caps {
            let (alias, value) = next(&mut names, attr);
            conditions.push(format!("{alias} <= {value}"));
            values.insert(
                value,
                AttributeValue::N((max - self.pending_delta(attr)).to_string()),
            );
        }
        for (attr, expected) in &self.equals {
            let (alias, value) = next(&mut names, attr);
            conditions.push(format!("{alias} = {value}"));
            values.insert(value, expected.clone());
        }
        let mut expression = String::new();
        if !sets.is_empty() {
            expression.push_str(&format!("SET {} ", sets.join(", ")));
        }
        expression.push_str(&format!("ADD {}", adds.join(", ")));
        if !removes.is_empty() {
            expression.push_str(&format!(" REMOVE {}", removes.join(", ")));
        }
        UpdateOp::builder()
            .table_name(table)
            .set_key(Some(key.attributes()))
            .update_expression(expression)
            .condition_expression(conditions.join(" AND "))
            .set_expression_attribute_names(Some(names))
            .set_expression_attribute_values(Some(values))
            .build()
            .map_err(dynamo_error)
    }
}

#[derive(Debug, Clone)]
enum PutMode {
    Insert,
    Replace(i64),
}

#[derive(Debug, Clone)]
enum Op {
    None,
    Put(Item, PutMode),
    Delete(Option<i64>),
    Update(Update),
    Check(i64),
}

#[derive(Debug, Clone)]
struct Slot {
    base: Option<Item>,
    read: bool,
    op: Op,
}

impl Slot {
    fn unread() -> Self {
        Self {
            base: None,
            read: false,
            op: Op::None,
        }
    }

    fn view(&self) -> Option<Item> {
        match &self.op {
            Op::None | Op::Check(_) => self.base.clone(),
            Op::Put(item, _) => Some(item.clone()),
            Op::Delete(_) => None,
            Op::Update(update) => self.base.clone().map(|mut item| {
                update.apply(&mut item);
                item
            }),
        }
    }

    fn base_version(&self) -> Option<i64> {
        self.base.as_ref().map(|item| {
            item.get(VERSION)
                .and_then(|v| v.as_n().ok())
                .and_then(|n| n.parse().ok())
                .unwrap_or(0)
        })
    }
}

pub struct TenantTx {
    store: Store,
    tenant_id: Uuid,
    slots: BTreeMap<Key, Slot>,
    messages: Vec<Relayed>,
}

impl TenantTx {
    pub fn begin(store: &Store, tenant_id: Uuid) -> Self {
        Self {
            store: store.clone(),
            tenant_id,
            slots: BTreeMap::new(),
            messages: Vec::new(),
        }
    }

    pub fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    fn guard(&self, key: &Key) -> Result<(), DbError> {
        if key.table == Table::Audit {
            if key.pk == format!("T#{}", self.tenant_id) {
                return Ok(());
            }
        } else {
            let own = format!("T#{}", self.tenant_id);
            if key.pk == own || key.pk.starts_with(&format!("{own}#")) {
                return Ok(());
            }
            if !key.pk.starts_with("T#") && GLOBAL_PREFIXES.iter().any(|p| key.pk.starts_with(p)) {
                return Ok(());
            }
        }
        Err(DbError::CrossTenant {
            tenant: self.tenant_id,
            pk: key.pk.clone(),
        })
    }

    fn guard_mutable(&self, key: &Key) -> Result<(), DbError> {
        self.guard(key)?;
        if key.table == Table::Audit {
            return Err(DbError::AppendOnly(key.sk.clone()));
        }
        Ok(())
    }

    pub async fn get_item(&mut self, key: &Key) -> Result<Option<Item>, DbError> {
        self.guard(key)?;
        if let Some(slot) = self.slots.get(key)
            && slot.read
        {
            return Ok(slot.view());
        }
        let base = self.store.get_item(key).await?;
        let slot = self.slots.entry(key.clone()).or_insert_with(Slot::unread);
        slot.base = base;
        slot.read = true;
        Ok(slot.view())
    }

    pub async fn get<E: Entity>(&mut self, key: &Key) -> Result<Option<E>, DbError> {
        self.get_item(key)
            .await?
            .map(|item| E::from_item(&item))
            .transpose()
    }

    pub async fn require<E: Entity>(&mut self, key: &Key) -> Result<E, DbError> {
        self.get(key)
            .await?
            .ok_or_else(|| DbError::Missing(format!("{} {}/{}", E::KIND, key.pk, key.sk)))
    }

    pub async fn query_items(
        &mut self,
        table: Table,
        pk: &str,
        prefix: &str,
    ) -> Result<Vec<Item>, DbError> {
        let probe = Key {
            table,
            pk: pk.to_string(),
            sk: String::new(),
        };
        self.guard(&probe)?;
        let fetched = self
            .store
            .query_items(table, None, pk, &SortKey::prefix(prefix), true, None)
            .await?;
        let mut seen = BTreeSet::new();
        for item in fetched {
            let key = Key::of(table, &item)?;
            seen.insert(key.clone());
            let slot = self.slots.entry(key).or_insert_with(Slot::unread);
            if !slot.read {
                slot.base = Some(item);
                slot.read = true;
            }
        }
        let mut views = self
            .slots
            .range(
                Key {
                    table,
                    pk: pk.to_string(),
                    sk: prefix.to_string(),
                }..,
            )
            .take_while(|(key, _)| key.table == table && key.pk == pk && key.sk.starts_with(prefix))
            .filter(|(key, slot)| seen.contains(*key) || matches!(slot.op, Op::Put(..)))
            .filter_map(|(_, slot)| slot.view())
            .collect::<Vec<_>>();
        views.sort_by(|a, b| {
            let sk = |item: &Item| item.get("sk").and_then(|v| v.as_s().ok()).cloned();
            sk(a).cmp(&sk(b))
        });
        Ok(views)
    }

    pub async fn query<E: Entity>(&mut self, pk: &str, prefix: &str) -> Result<Vec<E>, DbError> {
        decode_kind(self.query_items(Table::Main, pk, prefix).await?)
    }

    pub async fn list<E: Entity>(
        &self,
        listing: &Listing,
        keep: impl Fn(&E) -> bool,
    ) -> Result<Vec<E>, DbError> {
        self.guard(&Key {
            table: listing.table,
            pk: listing.pk.clone(),
            sk: String::new(),
        })?;
        self.store.list(listing, keep).await
    }

    pub fn insert_item(&mut self, key: Key, mut item: Item) -> Result<(), DbError> {
        self.guard(&key)?;
        item.insert(VERSION.into(), AttributeValue::N("1".into()));
        let slot = self.slots.entry(key).or_insert_with(Slot::unread);
        slot.op = Op::Put(item, PutMode::Insert);
        Ok(())
    }

    pub fn insert<E: Entity>(&mut self, entity: &E) -> Result<(), DbError> {
        self.insert_item(entity.key(), entity.to_item())
    }

    pub fn put_item(&mut self, key: Key, mut item: Item) -> Result<(), DbError> {
        self.guard_mutable(&key)?;
        let slot = self
            .slots
            .get_mut(&key)
            .filter(|slot| slot.read || matches!(slot.op, Op::Put(_, PutMode::Insert)))
            .ok_or_else(|| DbError::Unread(format!("{}/{}", key.pk, key.sk)))?;
        let mode = match &slot.op {
            Op::Put(_, mode) => mode.clone(),
            Op::Update(_) => {
                return Err(DbError::Unread(format!(
                    "{}/{} has a pending update",
                    key.pk, key.sk
                )));
            }
            _ => match slot.base_version() {
                Some(version) => PutMode::Replace(version),
                None => PutMode::Insert,
            },
        };
        let version = match mode {
            PutMode::Insert => 1,
            PutMode::Replace(version) => version + 1,
        };
        item.insert(VERSION.into(), AttributeValue::N(version.to_string()));
        slot.op = Op::Put(item, mode);
        Ok(())
    }

    pub fn put<E: Entity>(&mut self, entity: &E) -> Result<(), DbError> {
        self.put_item(entity.key(), entity.to_item())
    }

    pub fn delete(&mut self, key: &Key) -> Result<(), DbError> {
        self.guard_mutable(key)?;
        let slot = self.slots.entry(key.clone()).or_insert_with(Slot::unread);
        slot.op = match (&slot.op, slot.read, slot.base_version()) {
            (Op::Put(_, PutMode::Insert), _, None) => Op::None,
            (_, true, Some(version)) => Op::Delete(Some(version)),
            (_, true, None) => Op::None,
            (_, false, _) => Op::Delete(None),
        };
        Ok(())
    }

    pub fn update(&mut self, key: &Key, update: Update) -> Result<(), DbError> {
        self.guard_mutable(key)?;
        let slot = self.slots.entry(key.clone()).or_insert_with(Slot::unread);
        match &mut slot.op {
            Op::Put(item, _) => update.apply(item),
            Op::Update(existing) => existing.merge(update),
            Op::None | Op::Check(_) => slot.op = Op::Update(update),
            Op::Delete(_) => {
                return Err(DbError::Missing(format!(
                    "{}/{} is deleted in this transaction",
                    key.pk, key.sk
                )));
            }
        }
        Ok(())
    }

    pub fn pending_delta(&self, key: &Key, name: &str) -> i64 {
        match self.slots.get(key).map(|slot| &slot.op) {
            Some(Op::Update(update)) => update.pending_delta(name),
            _ => 0,
        }
    }

    pub fn assert_unchanged(&mut self, key: &Key) -> Result<(), DbError> {
        self.guard(key)?;
        let slot = self
            .slots
            .get_mut(key)
            .filter(|slot| slot.read)
            .ok_or_else(|| DbError::Unread(format!("{}/{}", key.pk, key.sk)))?;
        if let (Op::None, Some(version)) = (&slot.op, slot.base_version()) {
            slot.op = Op::Check(version);
        }
        Ok(())
    }

    pub fn emit(&mut self, job: Job) -> Uuid {
        self.emit_envelope(Envelope::new(job), None)
    }

    pub fn emit_at(&mut self, job: Job, at: DateTime<Utc>) -> Uuid {
        self.emit_envelope(Envelope::new(job), Some(at))
    }

    fn emit_envelope(&mut self, envelope: Envelope, deliver_at: Option<DateTime<Utc>>) -> Uuid {
        let id = envelope.id;
        self.messages.push(Relayed::Send {
            envelope,
            deliver_at,
        });
        id
    }

    pub fn cancel_schedule(&mut self, message_id: Uuid) {
        self.messages.push(Relayed::Cancel {
            schedule: schedule_name(message_id),
        });
    }

    pub fn has_writes(&self) -> bool {
        !self.messages.is_empty()
            || self
                .slots
                .values()
                .any(|slot| !matches!(slot.op, Op::None | Op::Check(_)))
    }

    pub async fn commit(self) -> Result<(), DbError> {
        let store = self.store.clone();
        let mut writes = Vec::new();
        for (key, slot) in self.slots {
            let table = store.tables().name(key.table).to_string();
            let write = match slot.op {
                Op::None => continue,
                Op::Check(version) => TransactWriteItem::builder()
                    .condition_check(
                        ConditionCheck::builder()
                            .table_name(&table)
                            .set_key(Some(key.attributes()))
                            .condition_expression("#v = :v")
                            .expression_attribute_names("#v", VERSION)
                            .expression_attribute_values(
                                ":v",
                                AttributeValue::N(version.to_string()),
                            )
                            .build()
                            .map_err(dynamo_error)?,
                    )
                    .build(),
                Op::Put(item, mode) => {
                    let item = store.dehydrate(&key.pk, item).await?;
                    let put = Put::builder().table_name(&table).set_item(Some(item));
                    let put = match mode {
                        PutMode::Insert => put
                            .condition_expression("attribute_not_exists(#pk)")
                            .expression_attribute_names("#pk", "pk"),
                        PutMode::Replace(version) => put
                            .condition_expression("#v = :v")
                            .expression_attribute_names("#v", VERSION)
                            .expression_attribute_values(
                                ":v",
                                AttributeValue::N(version.to_string()),
                            ),
                    };
                    TransactWriteItem::builder()
                        .put(put.build().map_err(dynamo_error)?)
                        .build()
                }
                Op::Delete(version) => {
                    let delete = Delete::builder()
                        .table_name(&table)
                        .set_key(Some(key.attributes()));
                    let delete = match version {
                        Some(version) => delete
                            .condition_expression("#v = :v")
                            .expression_attribute_names("#v", VERSION)
                            .expression_attribute_values(
                                ":v",
                                AttributeValue::N(version.to_string()),
                            ),
                        None => delete,
                    };
                    TransactWriteItem::builder()
                        .delete(delete.build().map_err(dynamo_error)?)
                        .build()
                }
                Op::Update(update) => TransactWriteItem::builder()
                    .update(update.build(&table, &key)?)
                    .build(),
            };
            writes.push(write);
        }
        let expires_at = Utc::now() + Duration::hours(MESSAGE_RETENTION_HOURS);
        for message in &self.messages {
            let id = Uuid::new_v4();
            let mut item = ItemWriter::new(MESSAGE_KIND);
            item.s("pk", format!("MSG#{id}"))
                .s("sk", "#")
                .s("body", serde_json::to_string(message)?)
                .n(TTL, expires_at.timestamp());
            writes.push(
                TransactWriteItem::builder()
                    .put(
                        Put::builder()
                            .table_name(&store.tables().main)
                            .set_item(Some(item.finish()))
                            .build()
                            .map_err(dynamo_error)?,
                    )
                    .build(),
            );
        }
        if writes.is_empty() {
            return Ok(());
        }
        store.transact(writes).await?;
        store.deliver(&self.messages).await
    }
}

pub async fn retry<T, F, Fut>(attempts: u32, mut operation: F) -> Result<T, DbError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, DbError>>,
{
    let mut attempt = 0;
    loop {
        attempt += 1;
        match operation().await {
            Err(DbError::Conflict) if attempt < attempts => {
                let jitter = u64::from(Uuid::new_v4().as_bytes()[0]) % 20;
                tokio::time::sleep(std::time::Duration::from_millis(
                    5 * u64::from(attempt) + jitter,
                ))
                .await;
            }
            other => return other,
        }
    }
}
