use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::item::{Key, format_time, padded};

pub fn tenant_pk(tenant_id: Uuid) -> String {
    format!("T#{tenant_id}")
}

pub fn task_pk(tenant_id: Uuid, task_id: Uuid) -> String {
    format!("T#{tenant_id}#TASK#{task_id}")
}

pub fn run_pk(tenant_id: Uuid, run_id: Uuid) -> String {
    format!("T#{tenant_id}#RUN#{run_id}")
}

pub fn config_pk(tenant_id: Uuid, config_id: Uuid) -> String {
    format!("T#{tenant_id}#CONFIG#{config_id}")
}

pub fn trigger_pk(tenant_id: Uuid, trigger_id: Uuid) -> String {
    format!("T#{tenant_id}#TRIGGER#{trigger_id}")
}

pub fn inbound_pk(tenant_id: Uuid) -> String {
    format!("T#{tenant_id}#INBOUND")
}

pub fn ref_key(tenant_id: Uuid, id: Uuid) -> Key {
    Key::main(format!("T#{tenant_id}#REF#{id}"), "#")
}

pub fn plans_pk() -> &'static str {
    "PLANS"
}

pub fn plan_key(plan_id: &str) -> Key {
    Key::main(plans_pk(), plan_id)
}

pub fn user_key(user_id: Uuid) -> Key {
    Key::main(format!("USER#{user_id}"), "#")
}

pub fn membership_key(user_id: Uuid, tenant_id: Uuid) -> Key {
    Key::main(format!("USER#{user_id}"), format!("MEMBER#{tenant_id}"))
}

pub fn uniq_key(kind: &str, value: &str) -> Key {
    Key::main(format!("UNIQ#{kind}#{value}"), "#")
}

pub fn invitation_email_pk(email: &str) -> String {
    format!("INVITEMAIL#{}", email.to_lowercase())
}

pub fn invitation_email_key(email: &str, tenant_id: Uuid, invitation_id: Uuid) -> Key {
    Key::main(
        invitation_email_pk(email),
        format!("INVITE#{tenant_id}#{invitation_id}"),
    )
}

pub fn tenant_key(tenant_id: Uuid) -> Key {
    Key::main(tenant_pk(tenant_id), "#TENANT")
}

pub fn member_key(tenant_id: Uuid, member_id: Uuid) -> Key {
    Key::main(tenant_pk(tenant_id), format!("MEMBER#{member_id}"))
}

pub fn invitation_key(tenant_id: Uuid, invitation_id: Uuid) -> Key {
    Key::main(tenant_pk(tenant_id), format!("INVITE#{invitation_id}"))
}

pub fn pending_invitation_key(tenant_id: Uuid, email: &str) -> Key {
    Key::main(
        tenant_pk(tenant_id),
        format!("INVITEPENDING#{}", email.to_lowercase()),
    )
}

pub fn credential_key(tenant_id: Uuid, credential_id: Uuid) -> Key {
    Key::main(tenant_pk(tenant_id), format!("CRED#{credential_id}"))
}

pub fn credential_name_key(tenant_id: Uuid, name: &str) -> Key {
    Key::main(tenant_pk(tenant_id), format!("CREDNAME#{name}"))
}

pub fn usage_snapshot_prefix(credential_id: Uuid) -> String {
    format!("USAGESNAP#{credential_id}#")
}

pub fn usage_snapshot_key(tenant_id: Uuid, credential_id: Uuid, captured_at: DateTime<Utc>) -> Key {
    Key::main(
        tenant_pk(tenant_id),
        format!(
            "{}{}",
            usage_snapshot_prefix(credential_id),
            format_time(captured_at)
        ),
    )
}

pub fn integration_key(tenant_id: Uuid, integration_id: Uuid) -> Key {
    Key::main(tenant_pk(tenant_id), format!("INTEG#{integration_id}"))
}

pub fn config_key(tenant_id: Uuid, config_id: Uuid) -> Key {
    Key::main(tenant_pk(tenant_id), format!("CONFIG#{config_id}"))
}

pub fn config_repository_key(tenant_id: Uuid, repository: &str, root_path: &str) -> Key {
    Key::main(
        tenant_pk(tenant_id),
        format!("CONFIGREPO#{repository}#{root_path}"),
    )
}

pub fn usage_alert_key(
    tenant_id: Uuid,
    period_start: DateTime<Utc>,
    metric: &str,
    threshold_percent: i64,
) -> Key {
    Key::main(
        tenant_pk(tenant_id),
        format!(
            "ALERT#{}#{metric}#{threshold_percent:03}",
            format_time(period_start)
        ),
    )
}

pub fn pending_task_key(tenant_id: Uuid, created_at: DateTime<Utc>, task_id: Uuid) -> Key {
    Key::main(
        tenant_pk(tenant_id),
        format!("PENDINGTASK#{}#{task_id}", format_time(created_at)),
    )
}

pub fn check_wait_marker_key(tenant_id: Uuid, wait_id: Uuid) -> Key {
    Key::main(tenant_pk(tenant_id), format!("CHECKWAIT#{wait_id}"))
}

pub fn revision_key(tenant_id: Uuid, config_id: Uuid, number: i64) -> Key {
    Key::main(
        config_pk(tenant_id, config_id),
        format!("REV#{}", padded(number)),
    )
}

pub fn revision_sha_key(tenant_id: Uuid, config_id: Uuid, sha: &str) -> Key {
    Key::main(config_pk(tenant_id, config_id), format!("REVSHA#{sha}"))
}

pub fn project_key(tenant_id: Uuid, config_id: Uuid, name: &str) -> Key {
    Key::main(config_pk(tenant_id, config_id), format!("PROJECT#{name}"))
}

pub fn repository_key(tenant_id: Uuid, config_id: Uuid, project: &str, name: &str) -> Key {
    Key::main(
        config_pk(tenant_id, config_id),
        format!("PROJECT#{project}#REPO#{name}"),
    )
}

pub fn session_key(tenant_id: Uuid, config_id: Uuid, name: &str) -> Key {
    Key::main(config_pk(tenant_id, config_id), format!("SESSION#{name}"))
}

pub fn skill_key(tenant_id: Uuid, config_id: Uuid, name: &str) -> Key {
    Key::main(config_pk(tenant_id, config_id), format!("SKILL#{name}"))
}

pub fn trigger_key(tenant_id: Uuid, config_id: Uuid, name: &str) -> Key {
    Key::main(config_pk(tenant_id, config_id), format!("TRIGGER#{name}"))
}

pub fn change_request_key(tenant_id: Uuid, config_id: Uuid, pr_number: i64) -> Key {
    Key::main(
        config_pk(tenant_id, config_id),
        format!("CR#{}", padded(pr_number)),
    )
}

pub fn trigger_id(config_id: Uuid, name: &str) -> Uuid {
    Uuid::new_v5(&config_id, format!("trigger:{name}").as_bytes())
}

pub fn config_entity_id(config_id: Uuid, kind: &str, name: &str) -> Uuid {
    Uuid::new_v5(&config_id, format!("{kind}:{name}").as_bytes())
}

pub fn trigger_event_key(tenant_id: Uuid, trigger_id: Uuid, key: &str) -> Key {
    Key::main(trigger_pk(tenant_id, trigger_id), format!("EVT#{key}"))
}

pub fn task_name_key(tenant_id: Uuid, trigger_id: Uuid, name: &str) -> Key {
    Key::main(trigger_pk(tenant_id, trigger_id), format!("TASK#{name}"))
}

pub fn task_key(tenant_id: Uuid, task_id: Uuid) -> Key {
    Key::main(task_pk(tenant_id, task_id), "#TASK")
}

pub fn dependency_key(tenant_id: Uuid, task_id: Uuid, blocked_by: Uuid) -> Key {
    Key::main(task_pk(tenant_id, task_id), format!("DEP#{blocked_by}"))
}

pub fn child_key(
    tenant_id: Uuid,
    parent_id: Uuid,
    created_at: DateTime<Utc>,
    child_id: Uuid,
) -> Key {
    Key::main(
        task_pk(tenant_id, parent_id),
        format!("CHILD#{}#{child_id}", format_time(created_at)),
    )
}

pub fn run_key(tenant_id: Uuid, task_id: Uuid, run_id: Uuid) -> Key {
    Key::main(task_pk(tenant_id, task_id), format!("RUN#{run_id}"))
}

pub fn workspace_key(tenant_id: Uuid, task_id: Uuid, workspace_id: Uuid) -> Key {
    Key::main(task_pk(tenant_id, task_id), format!("WS#{workspace_id}"))
}

pub fn skill_run_prefix(number: i32) -> String {
    format!("SR#{}", padded(i64::from(number)))
}

pub fn skill_run_key(tenant_id: Uuid, task_id: Uuid, number: i32) -> Key {
    Key::main(task_pk(tenant_id, task_id), skill_run_prefix(number))
}

pub fn state_entry_key(tenant_id: Uuid, task_id: Uuid, number: i32, sequence: i64) -> Key {
    Key::main(
        task_pk(tenant_id, task_id),
        format!("{}#E#{}", skill_run_prefix(number), padded(sequence)),
    )
}

pub fn output_key(
    tenant_id: Uuid,
    task_id: Uuid,
    number: i32,
    kind: &str,
    repository: &str,
    output_number: i64,
) -> Key {
    Key::main(
        task_pk(tenant_id, task_id),
        format!(
            "{}#OUT#{kind}#{repository}#{}",
            skill_run_prefix(number),
            padded(output_number)
        ),
    )
}

pub fn invocation_key(tenant_id: Uuid, task_id: Uuid, number: i32, idempotency_key: &str) -> Key {
    Key::main(
        task_pk(tenant_id, task_id),
        format!("{}#INV#{idempotency_key}", skill_run_prefix(number)),
    )
}

pub fn check_wait_key(tenant_id: Uuid, task_id: Uuid, number: i32, task_token: &str) -> Key {
    Key::main(
        task_pk(tenant_id, task_id),
        format!("{}#CHECK#{task_token}", skill_run_prefix(number)),
    )
}

pub fn approval_wait_key(tenant_id: Uuid, task_id: Uuid, number: i32, task_token: &str) -> Key {
    Key::main(
        task_pk(tenant_id, task_id),
        format!("{}#APPROVAL#{task_token}", skill_run_prefix(number)),
    )
}

pub fn timer_prefix(number: i32) -> String {
    format!("{}#TIMER#", skill_run_prefix(number))
}

pub fn timer_key(tenant_id: Uuid, task_id: Uuid, number: i32, timer_id: Uuid) -> Key {
    Key::main(
        task_pk(tenant_id, task_id),
        format!("{}{timer_id}", timer_prefix(number)),
    )
}

pub fn held_message_key(
    tenant_id: Uuid,
    task_id: Uuid,
    created_at: DateTime<Utc>,
    dedupe_key: &str,
) -> Key {
    Key::main(
        task_pk(tenant_id, task_id),
        format!("HELD#{}#{dedupe_key}", format_time(created_at)),
    )
}

pub fn usage_key(tenant_id: Uuid, task_id: Uuid, recorded_at: DateTime<Utc>, id: Uuid) -> Key {
    Key::main(
        task_pk(tenant_id, task_id),
        format!("USAGE#{}#{id}", format_time(recorded_at)),
    )
}

pub fn chat_message_key(
    tenant_id: Uuid,
    task_id: Uuid,
    created_at: DateTime<Utc>,
    id: Uuid,
) -> Key {
    Key::main(
        task_pk(tenant_id, task_id),
        format!("CHAT#{}#{id}", format_time(created_at)),
    )
}

pub fn inbound_event_key(tenant_id: Uuid, source: &str, key: &str) -> Key {
    Key::main(inbound_pk(tenant_id), format!("EVT#{source}#{key}"))
}

pub fn pending_inbound_key(tenant_id: Uuid, received_at: DateTime<Utc>, id: Uuid) -> Key {
    Key::main(
        inbound_pk(tenant_id),
        format!("PEND#{}#{id}", format_time(received_at)),
    )
}

pub fn run_channel_key(tenant_id: Uuid, run_id: Uuid) -> Key {
    Key::main(run_pk(tenant_id, run_id), "#CHANNEL")
}

pub fn run_event_key(
    tenant_id: Uuid,
    run_id: Uuid,
    seq: Option<i64>,
    received_at: DateTime<Utc>,
    id: Uuid,
) -> Key {
    let sk = match seq {
        Some(seq) => format!("EVT#{seq:020}"),
        None => format!("EVT#~{}#{id}", format_time(received_at)),
    };
    Key::main(run_pk(tenant_id, run_id), sk)
}

pub fn pending_run_event_key(
    tenant_id: Uuid,
    run_id: Uuid,
    received_at: DateTime<Utc>,
    id: Uuid,
) -> Key {
    Key::main(
        run_pk(tenant_id, run_id),
        format!("PEND#{}#{id}", format_time(received_at)),
    )
}

pub fn outbox_key(tenant_id: Uuid, run_id: Uuid, seq: i64) -> Key {
    Key::main(run_pk(tenant_id, run_id), format!("OUT#{}", padded(seq)))
}

pub fn outbox_dedupe_key(tenant_id: Uuid, run_id: Uuid, dedupe_key: &str) -> Key {
    Key::main(run_pk(tenant_id, run_id), format!("OUTKEY#{dedupe_key}"))
}

pub fn channel_instance_key(tenant_id: Uuid, run_id: Uuid, instance_id: Uuid) -> Key {
    Key::main(run_pk(tenant_id, run_id), format!("INST#{instance_id}"))
}

pub fn run_log_key(tenant_id: Uuid, run_id: Uuid, seq: i64) -> Key {
    Key::main(run_pk(tenant_id, run_id), format!("LOG#{seq:020}"))
}

pub fn audit_key(tenant_id: Uuid, at: DateTime<Utc>, id: Uuid) -> Key {
    Key::audit(tenant_pk(tenant_id), format!("{}#{id}", format_time(at)))
}

pub const TENANTS_INDEX: &str = "TENANTS";
pub const SWEEP_CONFIGS: &str = "SWEEP#CONFIG";
pub const SWEEP_CHECKS: &str = "SWEEP#CHECKWAIT";
pub const SWEEP_ACTIVE_TASKS: &str = "SWEEP#ACTIVETASK";

pub fn tasks_index(tenant_id: Uuid) -> String {
    format!("T#{tenant_id}#TASKS")
}

pub fn task_name_index(tenant_id: Uuid, name: &str) -> String {
    format!("T#{tenant_id}#TASKNAME#{name}")
}

pub fn runs_index(tenant_id: Uuid) -> String {
    format!("T#{tenant_id}#RUNS")
}

pub fn open_runs_index(tenant_id: Uuid) -> String {
    format!("T#{tenant_id}#OPENRUNS")
}

pub fn workspaces_index(tenant_id: Uuid) -> String {
    format!("T#{tenant_id}#WORKSPACES")
}

pub fn trigger_events_index(tenant_id: Uuid) -> String {
    format!("T#{tenant_id}#TRIGGEREVENTS")
}

pub fn skill_runs_index(tenant_id: Uuid, skill_name: &str) -> String {
    format!("T#{tenant_id}#SKILLRUNS#{skill_name}")
}

pub fn outputs_index(tenant_id: Uuid) -> String {
    format!("T#{tenant_id}#OUTPUTS")
}

pub fn usage_index(tenant_id: Uuid) -> String {
    format!("T#{tenant_id}#USAGE")
}

pub fn thread_index(tenant_id: Uuid, source: &str, thread: &str) -> String {
    format!("T#{tenant_id}#THREAD#{source}#{thread}")
}

pub fn time_id(at: DateTime<Utc>, id: Uuid) -> String {
    format!("{}#{id}", format_time(at))
}
