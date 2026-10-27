use chrono::{Duration, Utc};
use sanchoris_core::db::{
    self, DbError, NewTask, NewTriggerEvent, Recorded, TenantTx, TriggerEventOutcome,
};
use sanchoris_core::model::{NamedDefinition, TaskDefinition};
use sanchoris_core::testing::{APP_ROLE, SeededTenant, TestPostgres};
use serde_json::json;
use sqlx::{Connection, PgPool};
use uuid::Uuid;

fn definition() -> TaskDefinition {
    TaskDefinition {
        trigger: NamedDefinition {
            name: "labeled-issue".into(),
            definition: json!({ "State": "ENABLED" }),
        },
        session: NamedDefinition {
            name: "shop".into(),
            definition: json!({ "Worker": { "Agent": "claude", "Model": "claude-sonnet-5-5", "Credential": "acme-claude-team" } }),
        },
        skills: vec![NamedDefinition {
            name: "open-pr-green-ci".into(),
            definition: json!({ "StartAt": "Init" }),
        }],
    }
}

async fn create_task(
    pool: &PgPool,
    seeded: &SeededTenant,
    name: &str,
    blocked_by: &[Uuid],
) -> Recorded {
    let definition = definition();
    let mut tx = TenantTx::begin(pool, seeded.tenant_id).await.unwrap();
    let recorded = db::create_task(
        &mut tx,
        &NewTask {
            trigger_id: seeded.trigger_id,
            trigger_event_id: None,
            config_revision_id: seeded.config_revision_id,
            name,
            title: name,
            definition: &definition,
            parent_task_id: None,
            channel: None,
            requested_by_member_id: None,
            blocked_by,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    recorded
}

async fn dispatch(pool: &PgPool, tenant_id: Uuid) -> Vec<db::DispatchedTask> {
    let mut tx = TenantTx::begin(pool, tenant_id).await.unwrap();
    let dispatched = db::dispatch_pending_tasks(&mut tx).await.unwrap();
    tx.commit().await.unwrap();
    dispatched
}

async fn finish_task(admin: &PgPool, task_id: Uuid) {
    sqlx::query(
        "UPDATE sanchoris.tasks SET status = 'SUCCEEDED', finished_at = now() WHERE id = $1",
    )
    .bind(task_id)
    .execute(admin)
    .await
    .unwrap();
}

#[tokio::test]
async fn duplicate_trigger_events_are_recorded_once() {
    let pg = TestPostgres::start().await;
    let admin = pg.admin_pool().await;
    let app = pg.app_pool().await;
    let seeded = pg.seed_tenant(&admin, "pro").await;
    let event = json!({ "source": "github", "detail-type": "Issue Labeled", "detail": {} });
    let new_event = NewTriggerEvent {
        trigger_id: seeded.trigger_id,
        key: "delivery-1",
        event: &event,
    };

    let mut tx = TenantTx::begin(&app, seeded.tenant_id).await.unwrap();
    let first = db::record_trigger_event(&mut tx, &new_event).await.unwrap();
    tx.commit().await.unwrap();
    let mut tx = TenantTx::begin(&app, seeded.tenant_id).await.unwrap();
    let second = db::record_trigger_event(&mut tx, &new_event).await.unwrap();
    tx.commit().await.unwrap();

    assert!(matches!(first, Recorded::New(_)));
    assert_eq!(second, Recorded::Duplicate(first.id()));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM sanchoris.trigger_events")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(count, 1);

    let mut tx = TenantTx::begin(&app, seeded.tenant_id).await.unwrap();
    db::set_trigger_event_outcome(
        &mut tx,
        first.id(),
        &TriggerEventOutcome::Skipped {
            reason: "children limit".into(),
        },
    )
    .await
    .unwrap();
    let again =
        db::set_trigger_event_outcome(&mut tx, first.id(), &TriggerEventOutcome::Started).await;
    assert!(matches!(again, Err(DbError::TriggerEventNotPending(id)) if id == first.id()));
    tx.commit().await.unwrap();

    let (result, reason): (String, String) =
        sqlx::query_as("SELECT result, reason FROM sanchoris.trigger_events WHERE id = $1")
            .bind(first.id())
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(
        (result.as_str(), reason.as_str()),
        ("skipped", "children limit")
    );
}

#[tokio::test]
async fn concurrent_duplicate_deliveries_insert_one_event() {
    let pg = TestPostgres::start().await;
    let admin = pg.admin_pool().await;
    let app = pg.app_pool().await;
    let seeded = pg.seed_tenant(&admin, "pro").await;
    let event = json!({ "source": "github" });

    let attempts = (0..8).map(|_| {
        let app = app.clone();
        let event = event.clone();
        tokio::spawn(async move {
            let mut tx = TenantTx::begin(&app, seeded.tenant_id).await.unwrap();
            let recorded = db::record_trigger_event(
                &mut tx,
                &NewTriggerEvent {
                    trigger_id: seeded.trigger_id,
                    key: "delivery-race",
                    event: &event,
                },
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            recorded
        })
    });
    let mut results = Vec::new();
    for attempt in attempts.collect::<Vec<_>>() {
        results.push(attempt.await.unwrap());
    }
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Recorded::New(_)))
            .count(),
        1
    );
    let ids = results
        .iter()
        .map(|r| r.id())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), 1);
}

#[tokio::test]
async fn tasks_are_idempotent_by_trigger_and_name() {
    let pg = TestPostgres::start().await;
    let admin = pg.admin_pool().await;
    let app = pg.app_pool().await;
    let seeded = pg.seed_tenant(&admin, "pro").await;

    let first = create_task(&app, &seeded, "acme/shop-web#35", &[]).await;
    let second = create_task(&app, &seeded, "acme/shop-web#35", &[]).await;
    let other = create_task(&app, &seeded, "acme/shop-web#36", &[]).await;

    assert!(matches!(first, Recorded::New(_)));
    assert_eq!(second, Recorded::Duplicate(first.id()));
    assert!(matches!(other, Recorded::New(id) if id != first.id()));

    let (status, definition): (String, serde_json::Value) =
        sqlx::query_as("SELECT status, definition FROM sanchoris.tasks WHERE id = $1")
            .bind(first.id())
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(status, "PENDING");
    assert_eq!(definition["Session"]["Name"], "shop");
}

#[tokio::test]
async fn dispatch_respects_the_plan_concurrency() {
    let pg = TestPostgres::start().await;
    let admin = pg.admin_pool().await;
    let app = pg.app_pool().await;
    let seeded = pg.seed_tenant(&admin, "starter").await;
    let mut tasks = Vec::new();
    for n in 0..3 {
        tasks.push(
            create_task(&app, &seeded, &format!("task-{n}"), &[])
                .await
                .id(),
        );
    }

    let first = dispatch(&app, seeded.tenant_id).await;
    assert_eq!(
        first.iter().map(|d| d.task_id).collect::<Vec<_>>(),
        tasks[..2]
    );
    assert!(dispatch(&app, seeded.tenant_id).await.is_empty());

    let (session_name, worker, model, status): (String, String, String, String) = sqlx::query_as(
        "SELECT session_name, worker, model, status FROM sanchoris.runs WHERE id = $1",
    )
    .bind(first[0].run_id)
    .fetch_one(&admin)
    .await
    .unwrap();
    assert_eq!(
        (
            session_name.as_str(),
            worker.as_str(),
            model.as_str(),
            status.as_str()
        ),
        ("shop", "claude", "claude-sonnet-5-5", "creating")
    );

    finish_task(&admin, tasks[0]).await;
    let next = dispatch(&app, seeded.tenant_id).await;
    assert_eq!(
        next.iter().map(|d| d.task_id).collect::<Vec<_>>(),
        vec![tasks[2]]
    );

    let tasks_this_period: i32 =
        sqlx::query_scalar("SELECT tasks_this_period FROM sanchoris.tenants WHERE id = $1")
            .bind(seeded.tenant_id)
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(tasks_this_period, 3);
}

#[tokio::test]
async fn concurrent_dispatchers_never_exceed_the_cap() {
    let pg = TestPostgres::start().await;
    let admin = pg.admin_pool().await;
    let app = pg.app_pool().await;
    let seeded = pg.seed_tenant(&admin, "pro").await;
    for n in 0..12 {
        create_task(&app, &seeded, &format!("task-{n}"), &[]).await;
    }

    let handles = (0..6)
        .map(|_| {
            let app = app.clone();
            tokio::spawn(async move { dispatch(&app, seeded.tenant_id).await })
        })
        .collect::<Vec<_>>();
    let mut dispatched = Vec::new();
    for handle in handles {
        dispatched.extend(handle.await.unwrap());
    }

    assert_eq!(dispatched.len(), 5);
    let running: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sanchoris.tasks WHERE status = 'RUNNING'")
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(running, 5);
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM sanchoris.runs")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(runs, 5);
}

#[tokio::test]
async fn dispatch_waits_for_blocking_tasks() {
    let pg = TestPostgres::start().await;
    let admin = pg.admin_pool().await;
    let app = pg.app_pool().await;
    let seeded = pg.seed_tenant(&admin, "max").await;
    let blocker = create_task(&app, &seeded, "blocker", &[]).await.id();
    let blocked = create_task(&app, &seeded, "blocked", &[blocker]).await.id();

    let first = dispatch(&app, seeded.tenant_id).await;
    assert_eq!(
        first.iter().map(|d| d.task_id).collect::<Vec<_>>(),
        vec![blocker]
    );
    assert!(dispatch(&app, seeded.tenant_id).await.is_empty());

    finish_task(&admin, blocker).await;
    let next = dispatch(&app, seeded.tenant_id).await;
    assert_eq!(
        next.iter().map(|d| d.task_id).collect::<Vec<_>>(),
        vec![blocked]
    );
}

#[tokio::test]
async fn dispatch_stops_at_the_plan_task_limit_and_for_suspended_tenants() {
    let pg = TestPostgres::start().await;
    let admin = pg.admin_pool().await;
    let app = pg.app_pool().await;
    let free = pg.seed_tenant(&admin, "free").await;
    sqlx::query("UPDATE sanchoris.tenants SET tasks_this_period = 5 WHERE id = $1")
        .bind(free.tenant_id)
        .execute(&admin)
        .await
        .unwrap();
    create_task(&app, &free, "over-limit", &[]).await;
    assert!(dispatch(&app, free.tenant_id).await.is_empty());

    let suspended = pg.seed_tenant(&admin, "pro").await;
    sqlx::query("UPDATE sanchoris.tenants SET status = 'suspended', suspended_reason = 'abuse' WHERE id = $1")
        .bind(suspended.tenant_id)
        .execute(&admin)
        .await
        .unwrap();
    create_task(&app, &suspended, "suspended", &[]).await;
    assert!(dispatch(&app, suspended.tenant_id).await.is_empty());

    let worker = pg.worker_pool().await;
    let mut conn = worker.acquire().await.unwrap();
    let mut pending = db::tenants_with_pending_tasks(&mut conn).await.unwrap();
    pending.sort();
    let mut expected = vec![free.tenant_id, suspended.tenant_id];
    expected.sort();
    assert_eq!(pending, expected);
}

#[tokio::test]
async fn due_jobs_are_claimed_with_skip_locked() {
    let pg = TestPostgres::start().await;
    let admin = pg.admin_pool().await;
    let app = pg.app_pool().await;
    let worker = pg.worker_pool().await;
    let seeded = pg.seed_tenant(&admin, "pro").await;
    let now = Utc::now();
    let mut tx = TenantTx::begin(&app, seeded.tenant_id).await.unwrap();
    let mut job_refs = Vec::new();
    for offset in [30, 20, 10] {
        let ref_id = Uuid::new_v4();
        db::schedule_due_job(
            &mut tx,
            "skill_run",
            ref_id,
            now - Duration::seconds(offset),
        )
        .await
        .unwrap();
        job_refs.push(ref_id);
    }
    db::schedule_due_job(
        &mut tx,
        "skill_run",
        Uuid::new_v4(),
        now + Duration::hours(1),
    )
    .await
    .unwrap();
    db::schedule_due_job(
        &mut tx,
        "task_resume",
        Uuid::new_v4(),
        now - Duration::seconds(5),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let mut first = worker.begin().await.unwrap();
    let claimed_first = db::claim_due_jobs(&mut first, "skill_run", now, 1)
        .await
        .unwrap();
    assert_eq!(claimed_first.len(), 1);
    assert_eq!(claimed_first[0].ref_id, job_refs[0]);
    assert_eq!(claimed_first[0].tenant_id, seeded.tenant_id);

    let mut second = worker.begin().await.unwrap();
    let claimed_second = db::claim_due_jobs(&mut second, "skill_run", now, 10)
        .await
        .unwrap();
    assert_eq!(
        claimed_second.iter().map(|j| j.ref_id).collect::<Vec<_>>(),
        job_refs[1..]
    );

    db::complete_due_job(&mut first, claimed_first[0].id)
        .await
        .unwrap();
    first.commit().await.unwrap();
    for job in &claimed_second {
        db::retry_due_job(&mut second, job.id, now + Duration::minutes(5))
            .await
            .unwrap();
    }
    second.commit().await.unwrap();

    let mut third = worker.begin().await.unwrap();
    assert!(
        db::claim_due_jobs(&mut third, "skill_run", now, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let later = db::claim_due_jobs(&mut third, "skill_run", now + Duration::minutes(5), 10)
        .await
        .unwrap();
    assert_eq!(later.len(), 2);
    assert!(later.iter().all(|job| job.attempts == 1));
    third.rollback().await.unwrap();
}

#[tokio::test]
async fn row_level_security_isolates_tenants() {
    let pg = TestPostgres::start().await;
    let admin = pg.admin_pool().await;
    let app = pg.app_pool().await;
    let a = pg.seed_tenant(&admin, "pro").await;
    let b = pg.seed_tenant(&admin, "pro").await;
    let task_a = create_task(&app, &a, "shared-name", &[]).await.id();
    let task_b = create_task(&app, &b, "shared-name", &[]).await.id();

    let mut tx = TenantTx::begin(&app, a.tenant_id).await.unwrap();
    let visible: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM sanchoris.tasks")
        .fetch_all(tx.conn())
        .await
        .unwrap();
    assert_eq!(visible, vec![task_a]);
    let tenants: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM sanchoris.tenants")
        .fetch_all(tx.conn())
        .await
        .unwrap();
    assert_eq!(tenants, vec![a.tenant_id]);
    let updated = sqlx::query("UPDATE sanchoris.tasks SET title = 'stolen' WHERE id = $1")
        .bind(task_b)
        .execute(tx.conn())
        .await
        .unwrap();
    assert_eq!(updated.rows_affected(), 0);
    tx.rollback().await.unwrap();

    let mut tx = TenantTx::begin(&app, a.tenant_id).await.unwrap();
    let cross_insert = sqlx::query(
        "INSERT INTO sanchoris.due_jobs (tenant_id, kind, ref_id, due_at) VALUES ($1, 'skill_run', $2, now())",
    )
    .bind(b.tenant_id)
    .bind(Uuid::new_v4())
    .execute(tx.conn())
    .await
    .unwrap_err();
    assert!(
        cross_insert.to_string().contains("row-level security"),
        "{cross_insert}"
    );
    tx.rollback().await.unwrap();

    let mut fresh = sqlx::PgConnection::connect_with(&pg.connect_options(APP_ROLE))
        .await
        .unwrap();
    let unscoped = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sanchoris.tasks")
        .fetch_one(&mut fresh)
        .await
        .unwrap_err();
    assert!(unscoped.to_string().contains("app.tenant_id"), "{unscoped}");

    let worker = pg.worker_pool().await;
    let all: i64 = sqlx::query_scalar("SELECT count(*) FROM sanchoris.tasks")
        .fetch_one(&worker)
        .await
        .unwrap();
    assert_eq!(all, 2);
}

#[tokio::test]
async fn audit_events_are_append_only() {
    let pg = TestPostgres::start().await;
    let admin = pg.admin_pool().await;
    let app = pg.app_pool().await;
    let seeded = pg.seed_tenant(&admin, "pro").await;

    let mut tx = TenantTx::begin(&app, seeded.tenant_id).await.unwrap();
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO sanchoris.audit_events (tenant_id, actor_kind, actor_ref, action, target_kind, target_ref, summary) \
         VALUES ($1, 'member', $2, 'trigger.enable', 'trigger', 'labeled-issue', 'Enabled labeled-issue') RETURNING id",
    )
    .bind(seeded.tenant_id)
    .bind(seeded.member_id.to_string())
    .fetch_one(tx.conn())
    .await
    .unwrap();
    tx.commit().await.unwrap();

    for statement in [
        "UPDATE sanchoris.audit_events SET summary = 'changed' WHERE id = $1",
        "DELETE FROM sanchoris.audit_events WHERE id = $1",
    ] {
        let mut tx = TenantTx::begin(&app, seeded.tenant_id).await.unwrap();
        let denied = sqlx::query(statement)
            .bind(id)
            .execute(tx.conn())
            .await
            .unwrap_err();
        assert!(denied.to_string().contains("permission denied"), "{denied}");
        tx.rollback().await.unwrap();

        let rejected = sqlx::query(statement)
            .bind(id)
            .execute(&admin)
            .await
            .unwrap_err();
        assert!(rejected.to_string().contains("append-only"), "{rejected}");
    }
    let truncate = sqlx::query("TRUNCATE sanchoris.audit_events")
        .execute(&admin)
        .await
        .unwrap_err();
    assert!(truncate.to_string().contains("append-only"), "{truncate}");

    let summary: String =
        sqlx::query_scalar("SELECT summary FROM sanchoris.audit_events WHERE id = $1")
            .bind(id)
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(summary, "Enabled labeled-issue");
}

#[tokio::test]
async fn plans_are_read_only_for_the_app() {
    let pg = TestPostgres::start().await;
    let app = pg.app_pool().await;
    let concurrency: i32 =
        sqlx::query_scalar("SELECT max_concurrency FROM sanchoris.plans WHERE id = 'pro'")
            .fetch_one(&app)
            .await
            .unwrap();
    assert_eq!(concurrency, 5);
    let denied = sqlx::query("UPDATE sanchoris.plans SET max_concurrency = 100")
        .execute(&app)
        .await
        .unwrap_err();
    assert!(denied.to_string().contains("permission denied"), "{denied}");
}

#[tokio::test]
async fn schema_reapplies_cleanly() {
    let pg = TestPostgres::start().await;
    let admin = pg.admin_pool().await;
    sqlx::raw_sql(sanchoris_core::testing::SCHEMA_SQL)
        .execute(&admin)
        .await
        .unwrap();
}
