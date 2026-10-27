mod common;

use common::World;
use sanchoris_core::db::{TenantTx, UserTx};
use serde_json::{Value, json};
use uuid::Uuid;

#[tokio::test]
async fn members_of_one_tenant_cannot_read_another() {
    let w = World::new().await;

    let home = w.ok(&w.hana, "Home", json!({ "tenant": "acme" })).await;
    assert_eq!(home["tenant"], Value::Null);
    let shell = w.ok(&w.hana, "Shell", json!({})).await;
    let slugs = shell["viewer"]["memberships"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["tenant"]["slug"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(slugs, vec!["globex"]);
    let task = w
        .ok(
            &w.hana,
            "Task",
            json!({ "tenant": "globex", "id": w.issue_task }),
        )
        .await;
    assert_eq!(task["tenant"]["task"], Value::Null);
    let own = w
        .ok(
            &w.hana,
            "Task",
            json!({ "tenant": "globex", "id": w.globex_task }),
        )
        .await;
    assert_eq!(own["tenant"]["task"]["title"], "Globex secret task");
    let foreign = w
        .ok(
            &w.mika,
            "Task",
            json!({ "tenant": "acme", "id": w.globex_task }),
        )
        .await;
    assert_eq!(foreign["tenant"]["task"], Value::Null);

    assert_eq!(
        w.error_code(
            &w.hana,
            "ResumeTask",
            json!({ "input": { "taskId": w.issue_task, "message": "hi" } }),
        )
        .await,
        "NOT_FOUND"
    );
    assert_eq!(
        w.error_code(&w.hana, "StopTask", json!({ "taskId": w.issue_task }))
            .await,
        "NOT_FOUND"
    );
    assert_eq!(
        w.error_code(
            &w.hana,
            "SetTriggerState",
            json!({ "triggerId": w.labeled_trigger, "state": "DISABLED" }),
        )
        .await,
        "NOT_FOUND"
    );
    assert_eq!(
        w.error_code(
            &w.hana,
            "InviteMember",
            json!({ "input": { "tenantId": w.acme, "email": "x@globex.dev", "role": "MEMBER" } }),
        )
        .await,
        "NOT_FOUND"
    );
    assert_eq!(
        w.error_code(
            &w.hana,
            "RemoveCredential",
            json!({ "credentialId": w.org_credential })
        )
        .await,
        "NOT_FOUND"
    );
    assert_eq!(
        w.scalar::<String>(
            "SELECT status FROM sanchoris.tasks WHERE id = $1",
            w.issue_task
        )
        .await,
        "NEEDS_YOU"
    );
}

#[tokio::test]
async fn row_level_security_hides_other_tenants_from_the_app_role() {
    let w = World::new().await;
    let mut tx = TenantTx::begin(&w.app, w.globex).await.unwrap();
    let visible: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM sanchoris.tasks")
        .fetch_all(tx.conn())
        .await
        .unwrap();
    assert_eq!(visible, vec![w.globex_task]);
    let tenants: i64 = sqlx::query_scalar("SELECT count(*) FROM sanchoris.tenants")
        .fetch_one(tx.conn())
        .await
        .unwrap();
    assert_eq!(tenants, 1);
    drop(tx);

    let mut tx = UserTx::begin(&w.app, w.hana.user_id).await.unwrap();
    let tenants: Vec<String> =
        sqlx::query_scalar("SELECT slug FROM sanchoris.tenants ORDER BY slug")
            .fetch_all(tx.conn())
            .await
            .unwrap();
    assert_eq!(tenants, vec!["globex"]);
    let tasks: i64 = sqlx::query_scalar("SELECT count(*) FROM sanchoris.tasks")
        .fetch_one(tx.conn())
        .await
        .unwrap();
    assert_eq!(tasks, 0);
    let members: i64 = sqlx::query_scalar("SELECT count(*) FROM sanchoris.members")
        .fetch_one(tx.conn())
        .await
        .unwrap();
    assert_eq!(members, 1);
    drop(tx);

    let mut tx = UserTx::begin(&w.app, w.ren.user_id).await.unwrap();
    let invitations: i64 = sqlx::query_scalar("SELECT count(*) FROM sanchoris.invitations")
        .fetch_one(tx.conn())
        .await
        .unwrap();
    assert_eq!(invitations, 1);
    let tenants: i64 = sqlx::query_scalar("SELECT count(*) FROM sanchoris.tenants")
        .fetch_one(tx.conn())
        .await
        .unwrap();
    assert_eq!(tenants, 0);
    drop(tx);

    let mut tx = UserTx::begin(&w.app, w.operator.user_id).await.unwrap();
    let tenants: i64 = sqlx::query_scalar("SELECT count(*) FROM sanchoris.tenants")
        .fetch_one(tx.conn())
        .await
        .unwrap();
    assert_eq!(tenants, 2);
    let tasks: i64 = sqlx::query_scalar("SELECT count(*) FROM sanchoris.tasks")
        .fetch_one(tx.conn())
        .await
        .unwrap();
    assert_eq!(tasks, 0);
}

#[tokio::test]
async fn roles_gate_admin_and_owner_operations() {
    let w = World::new().await;
    let tenant = json!({ "tenant": "acme" });

    assert_eq!(
        w.error_code(
            &w.ken,
            "SetTriggerState",
            json!({ "triggerId": w.labeled_trigger, "state": "DISABLED" })
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(
            &w.ken,
            "InviteMember",
            json!({ "input": { "tenantId": w.acme, "email": "new@acme.dev", "role": "MEMBER" } }),
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(
            &w.ken,
            "AuditLog",
            json!({ "tenant": "acme", "filter": null })
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(
            &w.ken,
            "AddConfig",
            json!({ "input": { "tenantId": w.acme, "repository": "acme/x", "root": "/", "branch": "main" } }),
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(
            &w.ken,
            "AddCredential",
            json!({ "input": { "tenantId": w.acme, "name": "org-key", "provider": "ANTHROPIC", "key": "sk-ant-api03-0123456789", "scope": "ORGANIZATION" } }),
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(
            &w.ken,
            "RemoveCredential",
            json!({ "credentialId": w.org_credential })
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(
            &w.ken,
            "ChangeMemberRole",
            json!({ "memberId": w.ken.member_id, "role": "ADMIN" })
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(
            &w.sota,
            "InviteMember",
            json!({ "input": { "tenantId": w.acme, "email": "new@acme.dev", "role": "ADMIN" } }),
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(
            &w.sota,
            "ChangeMemberRole",
            json!({ "memberId": w.ken.member_id, "role": "ADMIN" })
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(
            &w.sota,
            "ChangePlan",
            json!({ "tenantId": w.acme, "planId": "max" })
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(&w.sota, "OpenBillingPortal", json!({ "tenantId": w.acme }))
            .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(
            &w.mika,
            "SuspendTenant",
            json!({ "tenantId": w.acme, "reason": "abuse" })
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(
            &w.mika,
            "ChangeMemberRole",
            json!({ "memberId": w.mika.member_id, "role": "MEMBER" })
        )
        .await,
        "CONFLICT"
    );

    let operator = w.ok(&w.mika, "OperatorTenants", json!({})).await;
    assert_eq!(operator["operator"], Value::Null);
    let configs = w.ok(&w.ken, "Configs", tenant.clone()).await;
    assert_eq!(configs["tenant"]["configs"].as_array().unwrap().len(), 1);
    let members = w.ok(&w.ken, "Members", tenant.clone()).await;
    assert_eq!(members["tenant"]["viewerMember"]["role"], "MEMBER");

    let response = w
        .inline(
            &w.operator,
            "query { operator { tenants { name tasks { id } } } }",
            json!({}),
        )
        .await;
    assert_eq!(
        response.errors[0]
            .extensions
            .as_ref()
            .unwrap()
            .get("code")
            .unwrap()
            .to_string(),
        "\"FORBIDDEN\""
    );

    let response = w
        .inline(
            &w.ren,
            "query { viewer { invitations { invitedBy { tenant { name tasks { id } } } } } }",
            json!({}),
        )
        .await;
    assert_eq!(
        response.errors[0]
            .extensions
            .as_ref()
            .unwrap()
            .get("code")
            .unwrap()
            .to_string(),
        "\"FORBIDDEN\""
    );

    assert!(w.audit_actions(w.acme).await.is_empty());
}

#[tokio::test]
async fn requests_without_a_user_are_rejected() {
    let w = World::new().await;
    let response = w
        .schema
        .execute(async_graphql::Request::new(
            "query { viewer { isOperator } }",
        ))
        .await;
    assert_eq!(
        response.errors[0]
            .extensions
            .as_ref()
            .unwrap()
            .get("code")
            .unwrap()
            .to_string(),
        "\"UNAUTHENTICATED\""
    );
}
