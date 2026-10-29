use std::collections::BTreeMap;
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration as StdDuration, Instant};

use aws_sdk_dynamodb::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_dynamodb::types::AttributeValue;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::clock::Clock;
use crate::db::{REF_CONFIG_REVISION, REF_TRIGGER, insert_ref};
use crate::item::{Entity, Item, KIND, Table};
use crate::items::{
    CONFIG_ACTIVE, Config, ConfigRevision, Member, Membership, Tenant, Trigger, User,
};
use crate::keys;
use crate::local::{MemoryBlobStore, MemoryQueue, MemoryScheduler};
use crate::schema::{create_tables, drop_tables, plans, seed_plans};
use crate::store::{Relay, Store, Tables};

pub const SEED_CONFIG_REPOSITORY_ID: i64 = 7001;

static SERVER: OnceLock<u16> = OnceLock::new();

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .expect("reserve a local port")
}

const SHARED_SERVER_PORTS_VAR: &str = "SANCHORIS_TEST_DYNAMO_PORTS";
const NEXTEST_SLOT_VAR: &str = "NEXTEST_TEST_GLOBAL_SLOT";

fn shared_server_port(ports: &str) -> u16 {
    let ports: Vec<u16> = ports
        .split(',')
        .map(|port| {
            port.parse().unwrap_or_else(|_| {
                panic!("{SHARED_SERVER_PORTS_VAR} has a value that is not a port: {port}")
            })
        })
        .collect();
    let slot = match std::env::var(NEXTEST_SLOT_VAR) {
        Ok(slot) => slot
            .parse::<usize>()
            .unwrap_or_else(|_| panic!("{NEXTEST_SLOT_VAR} is not a number: {slot}")),
        Err(_) => 0,
    };
    ports[slot % ports.len()]
}

fn server_port() -> u16 {
    *SERVER.get_or_init(|| {
        if let Ok(ports) = std::env::var(SHARED_SERVER_PORTS_VAR) {
            return shared_server_port(&ports);
        }
        let port = free_port();
        let parent = std::process::id();
        let script = format!(
            "dynamodb-local -inMemory -disableTelemetry -port {port} & child=$!; \
             while kill -0 {parent} 2>/dev/null; do sleep 1; done; kill $child"
        );
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("dynamodb-local is on PATH (run inside `nix develop`)");
        std::thread::spawn(move || child.wait());
        let deadline = Instant::now() + StdDuration::from_secs(60);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(
                Instant::now() < deadline,
                "DynamoDB Local did not listen on port {port}"
            );
            std::thread::sleep(StdDuration::from_millis(100));
        }
        port
    })
}

pub fn local_client(port: u16) -> aws_sdk_dynamodb::Client {
    let config = aws_sdk_dynamodb::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .endpoint_url(format!("http://127.0.0.1:{port}"))
        .credentials_provider(Credentials::new("test", "test", None, None, "test"))
        .build();
    aws_sdk_dynamodb::Client::from_conf(config)
}

pub struct TestDynamo {
    port: u16,
    client: aws_sdk_dynamodb::Client,
    tables: Tables,
    blobs: Arc<MemoryBlobStore>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeededTenant {
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub member_id: Uuid,
    pub config_id: Uuid,
    pub config_revision_id: Uuid,
    pub trigger_id: Uuid,
}

pub struct DirectStore {
    pub store: Store,
    pub queue: Arc<MemoryQueue>,
    pub scheduler: Arc<MemoryScheduler>,
}

impl TestDynamo {
    pub async fn start() -> Self {
        let port = server_port();
        let client = local_client(port);
        let suffix = Uuid::new_v4().simple().to_string();
        let tables = Tables {
            main: format!("sanchoris-test-{suffix}-main"),
            audit: format!("sanchoris-test-{suffix}-audit"),
        };
        let mut attempts = 0;
        loop {
            attempts += 1;
            match create_tables(&client, &tables).await {
                Ok(_) => break,
                Err(_) if attempts < 50 => {
                    tokio::time::sleep(StdDuration::from_millis(200)).await;
                }
                Err(error) => panic!("create test tables: {error}"),
            }
        }
        seed_plans(&client, &tables, &plans())
            .await
            .expect("seed plans");
        Self {
            port,
            client,
            tables,
            blobs: Arc::new(MemoryBlobStore::new()),
        }
    }

    pub fn endpoint_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn client(&self) -> &aws_sdk_dynamodb::Client {
        &self.client
    }

    pub fn tables(&self) -> &Tables {
        &self.tables
    }

    pub fn blobs(&self) -> &Arc<MemoryBlobStore> {
        &self.blobs
    }

    pub fn store(&self, relay: Relay) -> Store {
        Store::new(
            self.client.clone(),
            self.tables.clone(),
            self.blobs.clone(),
            relay,
        )
    }

    pub fn direct_store(&self, clock: Arc<dyn Clock>) -> DirectStore {
        let queue = Arc::new(MemoryQueue::new(clock.clone()));
        let scheduler = Arc::new(MemoryScheduler::new());
        let store = self.store(Relay::Direct {
            queue: queue.clone(),
            scheduler: scheduler.clone(),
            clock,
        });
        DirectStore {
            store,
            queue,
            scheduler,
        }
    }

    pub async fn set_stripe_price(&self, plan_id: &str, price_id: &str) {
        self.client
            .update_item()
            .table_name(&self.tables.main)
            .key("pk", AttributeValue::S(keys::plans_pk().into()))
            .key("sk", AttributeValue::S(plan_id.into()))
            .update_expression("SET stripe_price_id = :p")
            .expression_attribute_values(":p", AttributeValue::S(price_id.into()))
            .send()
            .await
            .expect("set the Stripe price");
    }

    pub async fn scan(&self, table: Table, kind: &str) -> Vec<Item> {
        let name = self.tables.name(table).to_string();
        let mut items = Vec::new();
        let mut start = None;
        loop {
            let output = self
                .client
                .scan()
                .table_name(&name)
                .filter_expression("#k = :k")
                .expression_attribute_names("#k", KIND)
                .expression_attribute_values(":k", AttributeValue::S(kind.into()))
                .consistent_read(true)
                .set_exclusive_start_key(start.take())
                .send()
                .await
                .expect("scan the test table");
            items.extend(output.items.unwrap_or_default());
            match output.last_evaluated_key {
                Some(key) if !key.is_empty() => start = Some(key),
                _ => return items,
            }
        }
    }

    pub async fn all<E: Entity>(&self) -> Vec<E> {
        let table = if E::KIND == "audit_event" {
            Table::Audit
        } else {
            Table::Main
        };
        self.scan(table, E::KIND)
            .await
            .iter()
            .map(|item| E::from_item(item).expect("test items decode"))
            .collect()
    }

    pub async fn put<E: Entity>(&self, entity: &E) {
        let mut item = entity.to_item();
        item.insert(crate::item::VERSION.into(), AttributeValue::N("1".into()));
        self.client
            .put_item()
            .table_name(&self.tables.main)
            .set_item(Some(item))
            .send()
            .await
            .expect("put a test item");
    }

    pub async fn put_raw(&self, mut item: Item) {
        item.insert(crate::item::VERSION.into(), AttributeValue::N("1".into()));
        self.client
            .put_item()
            .table_name(&self.tables.main)
            .set_item(Some(item))
            .send()
            .await
            .expect("put a raw test item");
    }

    pub async fn modify<E: Entity>(&self, key: &crate::item::Key, change: impl FnOnce(&mut E)) {
        let store = self.store(Relay::Stream);
        let item = store
            .get_item(key)
            .await
            .expect("read a test item")
            .expect("the test item exists");
        let version = item
            .get(crate::item::VERSION)
            .and_then(|v| v.as_n().ok())
            .and_then(|n| n.parse::<i64>().ok())
            .unwrap_or(0);
        let mut entity = E::from_item(&item).expect("test items decode");
        change(&mut entity);
        let mut item = entity.to_item();
        item.insert(
            crate::item::VERSION.into(),
            AttributeValue::N((version + 1).to_string()),
        );
        self.client
            .put_item()
            .table_name(self.tables.name(key.table))
            .set_item(Some(item))
            .send()
            .await
            .expect("replace a test item");
    }

    pub async fn seed_tenant(&self, plan_id: &str) -> SeededTenant {
        let store = self.store(Relay::Stream);
        let now = Utc::now();
        let user_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();
        let member_id = Uuid::new_v4();
        let config_id = Uuid::new_v4();
        let config_revision_id = Uuid::new_v4();
        let trigger_id = keys::trigger_id(config_id, "labeled-issue");
        let mut tx = store.tx(tenant_id);
        tx.insert(&User {
            id: user_id,
            cognito_subject: format!("subject-{user_id}"),
            email: format!("{user_id}@example.com"),
            display_name: "Test User".into(),
            github_user_id: None,
            github_login: None,
            is_operator: false,
            last_sign_in_at: None,
            created_at: now,
            updated_at: now,
        })
        .expect("seed user");
        tx.insert(&Tenant {
            id: tenant_id,
            slug: format!("tenant-{}", user_id.simple()),
            name: "Test tenant".into(),
            plan_id: plan_id.into(),
            period_start: now,
            tasks_this_period: 0,
            workspace_seconds_this_period: 0,
            live_workspaces: 0,
            status: "active".into(),
            suspended_reason: None,
            stripe_customer_id: None,
            created_by_user_id: user_id,
            created_at: now,
            updated_at: now,
        })
        .expect("seed tenant");
        tx.insert(&Member {
            id: member_id,
            tenant_id,
            user_id,
            role: "owner".into(),
            joined_at: now,
            last_active_at: None,
        })
        .expect("seed member");
        tx.insert(&Membership {
            user_id,
            tenant_id,
            member_id,
        })
        .expect("seed membership");
        tx.insert(&Config {
            id: config_id,
            tenant_id,
            repository_id: Some(SEED_CONFIG_REPOSITORY_ID),
            repository: "acme/sanchoris-config".into(),
            root_path: "/".into(),
            branch: "main".into(),
            status: CONFIG_ACTIVE.into(),
            active_revision_id: Some(config_revision_id),
            revisions: 1,
            repositories: BTreeMap::new(),
            synced_at: None,
            sync_error: None,
            created_at: now,
            updated_at: now,
        })
        .expect("seed config");
        let revision = ConfigRevision {
            id: config_revision_id,
            tenant_id,
            config_id,
            number: 1,
            commit_sha: "deadbeef".into(),
            parsed: Some(json!({})),
            validation_status: "valid".into(),
            validation_messages: json!([]),
            created_at: now,
        };
        tx.insert(&revision).expect("seed revision");
        insert_ref(
            &mut tx,
            config_revision_id,
            REF_CONFIG_REVISION,
            revision.key(),
            Some(config_id),
        )
        .expect("seed revision ref");
        let trigger = Trigger {
            id: trigger_id,
            tenant_id,
            config_id,
            config_revision_id,
            name: "labeled-issue".into(),
            state: "ENABLED".into(),
            event_pattern: Some(json!({ "source": ["github"] })),
            schedule_expression: None,
            input_transformer: None,
            start: json!({}),
            removed_at: None,
            next_run_at: None,
            schedule_message_id: None,
            next: None,
        };
        tx.insert(&trigger).expect("seed trigger");
        insert_ref(
            &mut tx,
            trigger_id,
            REF_TRIGGER,
            trigger.key(),
            Some(config_id),
        )
        .expect("seed trigger ref");
        tx.commit().await.expect("seed the tenant");
        SeededTenant {
            tenant_id,
            user_id,
            member_id,
            config_id,
            config_revision_id,
            trigger_id,
        }
    }
}

impl Drop for TestDynamo {
    fn drop(&mut self) {
        let port = self.port;
        let tables = self.tables.clone();
        let _ = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build a runtime to drop test tables");
            runtime.block_on(async {
                let _ = drop_tables(&local_client(port), &tables).await;
            });
        })
        .join();
    }
}
