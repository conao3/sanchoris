use crate::common;

use common::{Person, World};
use sanchoris_core::db::DbError;
use sanchoris_core::item::{Key, Table};
use sanchoris_core::items::{Member, Task, User};
use sanchoris_core::keys;
use sanchoris_core::model::TaskStatus;
use sanchoris_core::store::{Index, Listing};
use serde_json::{Value, json};
use uuid::Uuid;

struct Victim {
    tenant: Uuid,
    slug: &'static str,
    config: Uuid,
    trigger: Uuid,
    task: Uuid,
    run: Uuid,
    credential: Uuid,
    invitation: Uuid,
    member: Uuid,
}

fn acme(w: &World) -> Victim {
    Victim {
        tenant: w.acme,
        slug: "acme",
        config: w.config_id,
        trigger: w.labeled_trigger,
        task: w.issue_task,
        run: w.issue_run,
        credential: w.org_credential,
        invitation: w.invitation,
        member: w.ken.member_id.unwrap(),
    }
}

fn globex(w: &World) -> Victim {
    Victim {
        tenant: w.globex,
        slug: "globex",
        config: w.globex_config,
        trigger: w.globex_trigger,
        task: w.globex_task,
        run: w.globex_run,
        credential: w.globex_credential,
        invitation: w.globex_invitation,
        member: w.hana.member_id.unwrap(),
    }
}

fn foreign_mutations(victim: &Victim, own_tenant: Uuid) -> Vec<(&'static str, Value)> {
    vec![
        (
            "InviteMember",
            json!({ "input": { "tenantId": victim.tenant, "email": "x@example.dev", "role": "MEMBER" } }),
        ),
        (
            "RevokeInvitation",
            json!({ "invitationId": victim.invitation }),
        ),
        (
            "ChangeMemberRole",
            json!({ "memberId": victim.member, "role": "ADMIN" }),
        ),
        (
            "ResumeTask",
            json!({ "input": { "taskId": victim.task, "message": "hi" } }),
        ),
        (
            "SnoozeTask",
            json!({ "taskId": victim.task, "until": "2099-01-01T00:00:00Z" }),
        ),
        ("StopTask", json!({ "taskId": victim.task })),
        ("StopRun", json!({ "runId": victim.run })),
        (
            "SendChatMessage",
            json!({ "input": { "tenantId": victim.tenant, "scope": "all", "text": "hi" } }),
        ),
        (
            "SendChatMessage",
            json!({ "input": { "tenantId": own_tenant, "taskId": victim.task, "text": "hi" } }),
        ),
        (
            "SetTriggerState",
            json!({ "triggerId": victim.trigger, "state": "DISABLED" }),
        ),
        (
            "ProposeTrigger",
            json!({ "input": { "tenantId": victim.tenant, "configId": victim.config, "templateId": "labeled-issue", "repository": "acme/shop-web" } }),
        ),
        (
            "ProposeTrigger",
            json!({ "input": { "tenantId": own_tenant, "configId": victim.config, "templateId": "labeled-issue", "repository": "acme/shop-web" } }),
        ),
        (
            "AddConfig",
            json!({ "input": { "tenantId": victim.tenant, "repository": "x/y", "root": "/", "branch": "main" } }),
        ),
        ("SyncConfig", json!({ "configId": victim.config })),
        (
            "AddCredential",
            json!({ "input": { "tenantId": victim.tenant, "name": "x", "provider": "ANTHROPIC", "key": "sk-ant-api03-0123456789", "scope": "PERSONAL" } }),
        ),
        (
            "RemoveCredential",
            json!({ "credentialId": victim.credential }),
        ),
        (
            "ChangePlan",
            json!({ "tenantId": victim.tenant, "planId": "max" }),
        ),
        ("OpenBillingPortal", json!({ "tenantId": victim.tenant })),
        (
            "ExportAuditLog",
            json!({ "tenantId": victim.tenant, "filter": null }),
        ),
        ("ConnectSlack", json!({ "tenantId": victim.tenant })),
        (
            "DeleteTenant",
            json!({ "tenantId": victim.tenant, "slug": victim.slug }),
        ),
    ]
}

async fn assert_isolated(w: &World, attacker: &Person, own_tenant: Uuid, victim: &Victim) {
    let audit_before = w.audit_actions(victim.tenant).await;
    let tasks_before = w.all::<Task>().await.len();

    let home = w
        .ok(attacker, "Home", json!({ "tenant": victim.slug }))
        .await;
    assert_eq!(home["tenant"], Value::Null);
    let shell = w.ok(attacker, "Shell", json!({})).await;
    assert!(
        shell["viewer"]["memberships"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["tenant"]["slug"] != victim.slug)
    );
    let page = w.ok(attacker, "CreateTenantPage", json!({})).await;
    assert!(
        page["viewer"]["invitations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|invitation| invitation["id"] != victim.invitation.to_string())
    );
    let operator = w.ok(attacker, "OperatorTenants", json!({})).await;
    assert_eq!(operator["operator"], Value::Null);
    let own_slug = w
        .get::<sanchoris_core::items::Tenant>(&keys::tenant_key(own_tenant))
        .await
        .unwrap()
        .slug;
    let task = w
        .ok(
            attacker,
            "Task",
            json!({ "tenant": own_slug, "id": victim.task }),
        )
        .await;
    assert_eq!(task["tenant"]["task"], Value::Null);
    let chat = w
        .ok(
            attacker,
            "Chat",
            json!({ "tenant": own_slug, "task": victim.task, "withTask": true }),
        )
        .await;
    assert_eq!(chat["tenant"]["task"], Value::Null);

    for (operation, variables) in foreign_mutations(victim, own_tenant) {
        assert_eq!(
            w.error_code(attacker, operation, variables.clone()).await,
            "NOT_FOUND",
            "{operation} {variables}"
        );
    }
    let response = w
        .inline(
            attacker,
            "mutation($id: ID!) { removeMember(memberId: $id) }",
            json!({ "id": victim.member }),
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
        "\"NOT_FOUND\""
    );
    assert_eq!(
        w.error_code(
            attacker,
            "AcceptInvitation",
            json!({ "invitationId": victim.invitation })
        )
        .await,
        "NOT_FOUND"
    );

    assert_eq!(w.audit_actions(victim.tenant).await, audit_before);
    assert_eq!(w.all::<Task>().await.len(), tasks_before);
    assert_eq!(
        w.task(victim.tenant, victim.task).await.status,
        TaskStatus::NeedsYou
    );
    assert!(
        w.get::<Member>(&keys::member_key(victim.tenant, victim.member))
            .await
            .is_some()
    );
}

#[tokio::test]
async fn members_of_one_tenant_cannot_read_or_change_another() {
    let w = World::new().await;
    assert_isolated(&w, &w.hana, w.globex, &acme(&w)).await;
    assert_isolated(&w, &w.mika, w.acme, &globex(&w)).await;
    assert_isolated(&w, &w.sota, w.acme, &globex(&w)).await;

    let own = w
        .ok(
            &w.hana,
            "Task",
            json!({ "tenant": "globex", "id": w.globex_task }),
        )
        .await;
    assert_eq!(own["tenant"]["task"]["title"], "Globex secret task");
    let tasks = w
        .ok(
            &w.mika,
            "Tasks",
            json!({ "tenant": "acme", "filter": null }),
        )
        .await;
    assert!(
        tasks["tenant"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|task| task["id"] != w.globex_task.to_string())
    );
    let chats = w
        .ok(
            &w.mika,
            "Chat",
            json!({ "tenant": "acme", "task": w.chat_task, "withTask": false }),
        )
        .await;
    assert!(
        chats["tenant"]["chats"]
            .as_array()
            .unwrap()
            .iter()
            .all(|task| task["id"] != w.globex_task.to_string())
    );
    let members = w.ok(&w.mika, "Members", json!({ "tenant": "acme" })).await;
    assert_eq!(members["tenant"]["members"].as_array().unwrap().len(), 3);
    assert_eq!(members["tenant"]["usage"]["seatsUsed"], 4);
}

#[tokio::test]
async fn tenant_transactions_reject_keys_of_other_tenants() {
    let w = World::new().await;
    let mut tx = w.store.tx(w.globex);
    let foreign = keys::task_key(w.acme, w.issue_task);
    assert!(matches!(
        tx.get::<Task>(&foreign).await,
        Err(DbError::CrossTenant { .. })
    ));
    assert!(matches!(
        tx.query::<Task>(&keys::task_pk(w.acme, w.issue_task), "")
            .await,
        Err(DbError::CrossTenant { .. })
    ));
    assert!(matches!(
        tx.list::<Task>(
            &Listing::index(Index::Gsi1, keys::tasks_index(w.acme)),
            |_| true
        )
        .await,
        Err(DbError::CrossTenant { .. })
    ));
    assert!(matches!(
        tx.list::<sanchoris_core::items::AuditEvent>(
            &Listing::audit(keys::tenant_pk(w.acme)),
            |_| true
        )
        .await,
        Err(DbError::CrossTenant { .. })
    ));
    assert!(matches!(
        tx.list::<sanchoris_core::items::Tenant>(
            &Listing::index(Index::Gsi2, keys::TENANTS_INDEX),
            |_| true
        )
        .await,
        Err(DbError::CrossTenant { .. })
    ));
    assert!(matches!(
        tx.delete(&foreign),
        Err(DbError::CrossTenant { .. })
    ));
    let own = tx
        .list::<Task>(
            &Listing::index(Index::Gsi1, keys::tasks_index(w.globex)),
            |_| true,
        )
        .await
        .unwrap();
    assert_eq!(
        own.iter().map(|task| task.id).collect::<Vec<_>>(),
        vec![w.globex_task]
    );
    let user: Option<User> = tx.get(&keys::user_key(w.hana.user_id)).await.unwrap();
    assert!(user.is_some());
    let plans: Key = keys::plan_key("free");
    assert!(matches!(
        tx.delete(&plans),
        Err(DbError::CrossTenant { .. })
    ));
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
        w.error_code(&w.ken, "SyncConfig", json!({ "configId": w.config_id }))
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
            &w.ken,
            "RevokeInvitation",
            json!({ "invitationId": w.invitation })
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(&w.ken, "ConnectSlack", json!({ "tenantId": w.acme }))
            .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(
            &w.ken,
            "ExportAuditLog",
            json!({ "tenantId": w.acme, "filter": null })
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
        w.error_code(&w.mika, "UnsuspendTenant", json!({ "tenantId": w.acme }))
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

    assert_eq!(
        w.inline_code(
            &w.operator,
            "query { operator { tenants { name tasks { id } } } }",
            json!({}),
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.inline_code(
            &w.operator,
            "query { operator { tenants { members { id } } } }",
            json!({}),
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.inline_code(
            &w.ren,
            "query { viewer { invitations { invitedBy { tenant { name tasks { id } } } } } }",
            json!({}),
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.inline_code(
            &w.ren,
            "query { viewer { invitations { invitedBy { tenant { plan { id } } } } } }",
            json!({}),
        )
        .await,
        "FORBIDDEN"
    );

    assert!(w.audit_actions(w.acme).await.is_empty());
}

fn credential_ids(members: &Value, member_id: Uuid) -> Vec<String> {
    members["tenant"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["id"] == member_id.to_string())
        .unwrap()["credentials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|credential| credential["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn members_see_only_their_own_personal_credentials_unless_admin() {
    let w = World::new().await;
    let tenant = json!({ "tenant": "acme" });
    let mika = w.mika.member_id.unwrap();
    let by_ken = w.ok(&w.ken, "Members", tenant.clone()).await;
    assert!(credential_ids(&by_ken, mika).is_empty());
    let by_sota = w.ok(&w.sota, "Members", tenant.clone()).await;
    assert_eq!(
        credential_ids(&by_sota, mika),
        [w.mika_credential.to_string()]
    );
    let by_mika = w.ok(&w.mika, "Members", tenant).await;
    assert_eq!(
        credential_ids(&by_mika, mika),
        [w.mika_credential.to_string()]
    );
}

#[tokio::test]
async fn invitations_operators_and_roles_reject_the_wrong_person() {
    let w = World::new().await;
    for (who, invitation) in [
        (&w.ken, w.invitation),
        (&w.hana, w.invitation),
        (&w.ren, w.globex_invitation),
    ] {
        assert_eq!(
            w.error_code(
                who,
                "AcceptInvitation",
                json!({ "invitationId": invitation })
            )
            .await,
            "NOT_FOUND"
        );
    }
    assert_eq!(
        w.error_code(
            &w.hana,
            "SuspendTenant",
            json!({ "tenantId": w.acme, "reason": "abuse" })
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(&w.hana, "UnsuspendTenant", json!({ "tenantId": w.acme }))
            .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(
            &w.sota,
            "ChangeMemberRole",
            json!({ "memberId": w.sota.member_id, "role": "OWNER" })
        )
        .await,
        "FORBIDDEN"
    );
    let sota = w
        .get::<Member>(&keys::member_key(w.acme, w.sota.member_id.unwrap()))
        .await
        .unwrap();
    assert_eq!(sota.role, "admin");

    let home = w.ok(&w.operator, "Home", json!({ "tenant": "acme" })).await;
    assert_eq!(home["tenant"], Value::Null);
    for query in [
        "query { operator { tenants { credentials { id } } } }",
        "query { operator { tenants { configs { id } } } }",
        "query { operator { tenants { invitations { id } } } }",
    ] {
        assert_eq!(
            w.inline_code(&w.operator, query, json!({})).await,
            "FORBIDDEN",
            "{query}"
        );
    }
    let tenant: sanchoris_core::items::Tenant = w.get(&keys::tenant_key(w.acme)).await.unwrap();
    assert_eq!(tenant.status, "active");
    assert!(w.audit_actions(w.acme).await.is_empty());
}

#[tokio::test]
async fn requests_without_a_user_are_rejected() {
    let w = World::new().await;
    for query in [
        "query { viewer { isOperator } }",
        "query { plans { id } }",
        "query { tenant(slug: \"acme\") { id } }",
        "query { operator { liveWorkspaces } }",
        "query { triggerTemplates { id } }",
        "mutation { connectGitHub { url } }",
    ] {
        let response = w.schema.execute(async_graphql::Request::new(query)).await;
        assert_eq!(
            response.errors[0]
                .extensions
                .as_ref()
                .unwrap()
                .get("code")
                .unwrap()
                .to_string(),
            "\"UNAUTHENTICATED\"",
            "{query}"
        );
    }
}

#[tokio::test]
async fn storage_and_port_failures_reach_the_client_as_internal_errors() {
    let w = World::new().await;
    *w.recorder.plan_change_fails.lock().unwrap() = true;
    let failed = w
        .run(
            &w.mika,
            "ChangePlan",
            json!({ "tenantId": w.acme, "planId": "max" }),
        )
        .await;
    assert_eq!(failed.errors.len(), 1);
    assert_eq!(failed.errors[0].message, "internal error");
    assert_eq!(
        failed.errors[0].extensions.as_ref().unwrap().get("code"),
        Some(&async_graphql::Value::from("INTERNAL"))
    );

    let key = keys::tenant_key(w.acme);
    w.dynamo
        .client()
        .update_item()
        .table_name(w.dynamo.tables().name(Table::Main))
        .set_key(Some(key.attributes()))
        .update_expression("REMOVE #name")
        .expression_attribute_names("#name", "name")
        .send()
        .await
        .unwrap();
    let broken = w
        .inline(&w.mika, "{ tenant(slug: \"acme\") { name } }", json!({}))
        .await;
    assert_eq!(
        broken
            .errors
            .iter()
            .map(|error| error.message.as_str())
            .collect::<Vec<_>>(),
        vec!["internal error"]
    );
    let refused = w
        .inline(
            &w.mika,
            "mutation { changePlan(tenantId: \"nowhere\", planId: \"max\") { checkoutUrl } }",
            json!({}),
        )
        .await;
    assert_eq!(refused.errors[0].message, "nowhere is not a valid id");
}

#[tokio::test]
async fn deep_complex_and_introspection_queries_are_refused() {
    let w = World::new().await;
    let deep = format!(
        "{{ viewer {{ memberships {}id{} }} }}",
        "{ tenant { members ".repeat(6),
        " } }".repeat(6)
    );
    let complex = format!(
        "{{ {} }}",
        (0..sanchoris_graphql::MAX_COMPLEXITY)
            .map(|n| format!("p{n}: plans {{ id }}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let introspection = w
        .inline(&w.mika, "{ __schema { queryType { name } } }", json!({}))
        .await;
    assert_eq!(
        introspection.data.into_json().unwrap(),
        json!({ "__schema": null })
    );
    for (query, message) in [
        (deep.as_str(), "Query is nested too deep."),
        (complex.as_str(), "Query is too complex."),
    ] {
        let response = w.inline(&w.mika, query, json!({})).await;
        assert_eq!(
            response
                .errors
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>(),
            vec![message],
            "{query}"
        );
    }
}
