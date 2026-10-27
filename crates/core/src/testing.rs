use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tempfile::TempDir;
use uuid::Uuid;

pub const SCHEMA_SQL: &str = include_str!("../../../db/schema.sql");
pub const APP_ROLE: &str = "sanchoris_test_app";
pub const WORKER_ROLE: &str = "sanchoris_test_worker";
pub const ADMIN_ROLE: &str = "postgres";

pub struct TestPostgres {
    dir: TempDir,
    port: u16,
}

impl TestPostgres {
    pub async fn start() -> Self {
        let postgres = Self::start_cluster();
        let admin = postgres.pool(ADMIN_ROLE).await;
        sqlx::raw_sql(SCHEMA_SQL)
            .execute(&admin)
            .await
            .expect("apply db/schema.sql");
        sqlx::raw_sql(
            "CREATE ROLE sanchoris_test_app LOGIN IN ROLE sanchoris_app; \
             CREATE ROLE sanchoris_test_worker LOGIN IN ROLE sanchoris_worker;",
        )
        .execute(&admin)
        .await
        .expect("create test login roles");
        admin.close().await;
        postgres
    }

    pub fn start_cluster() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("sanchoris-test-postgres-")
            .tempdir()
            .expect("create a temporary directory");
        run(Command::new("initdb")
            .arg("--pgdata")
            .arg(data_dir(dir.path()))
            .args([
                "--username",
                ADMIN_ROLE,
                "--auth",
                "trust",
                "--encoding",
                "UTF8",
                "--locale",
                "C",
                "--no-sync",
            ]));
        std::fs::create_dir(socket_dir(dir.path())).expect("create the socket directory");
        let mut last_error = String::new();
        for _ in 0..5 {
            let port = free_port();
            let options = format!(
                "-p {port} -k {} -c listen_addresses=127.0.0.1 -c fsync=off -c synchronous_commit=off -c full_page_writes=off",
                socket_dir(dir.path()).display()
            );
            let output = Command::new("pg_ctl")
                .arg("--pgdata")
                .arg(data_dir(dir.path()))
                .arg("--log")
                .arg(dir.path().join("postgres.log"))
                .args(["--wait", "--options", &options, "start"])
                .output()
                .expect("pg_ctl is on PATH (run inside `nix develop`)");
            if output.status.success() {
                return Self { dir, port };
            }
            last_error = String::from_utf8_lossy(&output.stderr).into_owned();
        }
        panic!("pg_ctl start failed: {last_error}");
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn url(&self, role: &str) -> String {
        format!("postgres://{role}@127.0.0.1:{}/postgres", self.port)
    }

    pub fn connect_options(&self, role: &str) -> PgConnectOptions {
        PgConnectOptions::new()
            .host("127.0.0.1")
            .port(self.port)
            .username(role)
            .database("postgres")
    }

    pub async fn pool(&self, role: &str) -> PgPool {
        PgPoolOptions::new()
            .max_connections(8)
            .connect_with(self.connect_options(role))
            .await
            .expect("connect to the test cluster")
    }

    pub async fn admin_pool(&self) -> PgPool {
        self.pool(ADMIN_ROLE).await
    }

    pub async fn app_pool(&self) -> PgPool {
        self.pool(APP_ROLE).await
    }

    pub async fn worker_pool(&self) -> PgPool {
        self.pool(WORKER_ROLE).await
    }

    pub async fn seed_tenant(&self, admin: &PgPool, plan_id: &str) -> SeededTenant {
        let user_id = Uuid::new_v4();
        sqlx::query("INSERT INTO sanchoris.users (id, cognito_subject, email, display_name) VALUES ($1, $2, $3, 'Test User')")
            .bind(user_id)
            .bind(format!("subject-{user_id}"))
            .bind(format!("{user_id}@example.com"))
            .execute(admin)
            .await
            .expect("insert user");
        let tenant_id: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.tenants (name, plan_id, period_start, status, created_by_user_id) \
             VALUES ('Test tenant', $1, now(), 'active', $2) RETURNING id",
        )
        .bind(plan_id)
        .bind(user_id)
        .fetch_one(admin)
        .await
        .expect("insert tenant");
        let member_id: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.members (tenant_id, user_id, role) VALUES ($1, $2, 'owner') RETURNING id",
        )
        .bind(tenant_id)
        .bind(user_id)
        .fetch_one(admin)
        .await
        .expect("insert member");
        let config_id: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.configs (tenant_id, repository, root_path) VALUES ($1, 'acme/sanchoris-config', '/') RETURNING id",
        )
        .bind(tenant_id)
        .fetch_one(admin)
        .await
        .expect("insert config");
        let config_revision_id: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.config_revisions (tenant_id, config_id, commit_sha, parsed, validation_status) \
             VALUES ($1, $2, 'deadbeef', '{}'::jsonb, 'valid') RETURNING id",
        )
        .bind(tenant_id)
        .bind(config_id)
        .fetch_one(admin)
        .await
        .expect("insert config revision");
        let trigger_id: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.triggers (tenant_id, config_id, config_revision_id, name, state, event_pattern, start) \
             VALUES ($1, $2, $3, 'labeled-issue', 'ENABLED', '{\"source\": [\"github\"]}'::jsonb, '{}'::jsonb) RETURNING id",
        )
        .bind(tenant_id)
        .bind(config_id)
        .bind(config_revision_id)
        .fetch_one(admin)
        .await
        .expect("insert trigger");
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

impl Drop for TestPostgres {
    fn drop(&mut self) {
        let _ = Command::new("pg_ctl")
            .arg("--pgdata")
            .arg(data_dir(self.dir.path()))
            .args(["--mode", "immediate", "stop"])
            .output();
    }
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

fn data_dir(root: &Path) -> PathBuf {
    root.join("data")
}

fn socket_dir(root: &Path) -> PathBuf {
    root.join("socket")
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .expect("reserve a local port")
}

fn run(command: &mut Command) {
    let output = command.output().unwrap_or_else(|error| {
        panic!("{command:?} failed to start (run inside `nix develop`): {error}")
    });
    assert!(
        output.status.success(),
        "{command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
