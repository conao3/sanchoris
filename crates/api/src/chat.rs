use std::time::Duration;

use sqlx::{PgPool, Row};
use uuid::Uuid;

const INTERVAL: Duration = Duration::from_secs(1);
const BATCH: i64 = 50;

pub async fn relay_chat_posts(pool: &PgPool) -> Result<usize, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query(
        "SELECT o.id, o.tenant_id, o.run_id, o.payload, r.task_id FROM sanchoris.run_outbox o \
         JOIN sanchoris.runs r ON r.id = o.run_id \
         WHERE o.type = 'chat_post' AND o.delivered_at IS NULL \
         ORDER BY o.created_at, o.seq LIMIT $1 FOR UPDATE OF o SKIP LOCKED",
    )
    .bind(BATCH)
    .fetch_all(&mut *tx)
    .await?;
    for row in &rows {
        let payload: serde_json::Value = row.try_get("payload")?;
        let outbox_id: Uuid = row.try_get("id")?;
        sqlx::query(
            "INSERT INTO sanchoris.chat_messages (tenant_id, task_id, run_id, outbox_id, channel, author_kind, text) \
             VALUES ($1, $2, $3, $4, $5, 'agent', $6) ON CONFLICT (outbox_id) DO NOTHING",
        )
        .bind(row.try_get::<Uuid, _>("tenant_id")?)
        .bind(row.try_get::<Uuid, _>("task_id")?)
        .bind(row.try_get::<Uuid, _>("run_id")?)
        .bind(outbox_id)
        .bind(&payload["channel"])
        .bind(payload["text"].as_str())
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE sanchoris.run_outbox SET delivered_at = now() WHERE id = $1")
            .bind(outbox_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(rows.len())
}

pub async fn run(pool: PgPool, mut shutdown: tokio::sync::watch::Receiver<bool>) {
    let mut interval = tokio::time::interval(INTERVAL);
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = shutdown.changed() => return,
        }
        if let Err(error) = relay_chat_posts(&pool).await {
            eprintln!("sanchoris api: chat relay: {error}");
        }
    }
}
