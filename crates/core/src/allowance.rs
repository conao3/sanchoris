use crate::items::{Plan, Tenant};

/// The extra usage a pack adds to a tenant on a plan, for the plan's price of one period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsagePack {
    pub tasks: i64,
    pub workspace_seconds: i64,
    pub price_cents: i64,
}

/// The pack a tenant on `plan` can buy: one and a half periods of the plan's task and workspace-time limits for the price of one period. None for a free plan and a plan without both limits.
pub fn usage_pack(plan: &Plan) -> Option<UsagePack> {
    let pack = |limit: i64| limit * 3 / 2;
    match (
        plan.price_cents,
        plan.task_limit,
        plan.workspace_seconds_limit,
    ) {
        (price_cents, Some(tasks), Some(seconds)) if price_cents > 0 => Some(UsagePack {
            tasks: pack(tasks),
            workspace_seconds: pack(seconds),
            price_cents,
        }),
        _ => None,
    }
}

/// The tasks the tenant may start in the period: the plan's limit and the tenant's extra tasks on top. None when the plan has no limit.
pub fn task_allowance(tenant: &Tenant, plan: &Plan) -> Option<i64> {
    plan.task_limit.map(|limit| limit + tenant.extra_tasks)
}

/// The workspace seconds the tenant may use in the period: the plan's limit and the tenant's extra workspace seconds on top. None when the plan has no limit.
pub fn workspace_seconds_allowance(tenant: &Tenant, plan: &Plan) -> Option<i64> {
    plan.workspace_seconds_limit
        .map(|limit| limit + tenant.extra_workspace_seconds)
}

fn over(used: i64, limit: Option<i64>) -> i64 {
    limit.map_or(0, |limit| (used - limit).max(0))
}

/// The extra tasks the tenant has left: what the period used past the plan's limit comes off them.
pub fn extra_tasks_left(tenant: &Tenant, plan: &Plan) -> i64 {
    (tenant.extra_tasks - over(tenant.tasks_this_period, plan.task_limit)).max(0)
}

/// The extra workspace seconds the tenant has left: what the period used past the plan's limit comes off them.
pub fn extra_workspace_seconds_left(tenant: &Tenant, plan: &Plan) -> i64 {
    (tenant.extra_workspace_seconds
        - over(
            tenant.workspace_seconds_this_period,
            plan.workspace_seconds_limit,
        ))
    .max(0)
}
