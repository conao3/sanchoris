CREATE SCHEMA IF NOT EXISTS sanchoris;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sanchoris_app') THEN
        CREATE ROLE sanchoris_app NOLOGIN;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sanchoris_worker') THEN
        CREATE ROLE sanchoris_worker NOLOGIN;
    END IF;
END
$$;

CREATE OR REPLACE FUNCTION sanchoris.current_tenant_id() RETURNS uuid
    LANGUAGE sql STABLE
    AS $$ SELECT current_setting('app.tenant_id')::uuid $$;

CREATE OR REPLACE FUNCTION sanchoris.reject_audit_event_change() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    RAISE EXCEPTION 'sanchoris.audit_events is append-only (% rejected)', TG_OP
        USING ERRCODE = 'insufficient_privilege';
END
$$;

CREATE TABLE IF NOT EXISTS sanchoris.plans (
    id text PRIMARY KEY,
    name text NOT NULL UNIQUE,
    price_cents integer NOT NULL CHECK (price_cents >= 0),
    currency text NOT NULL,
    billing_unit text NOT NULL CHECK (billing_unit IN ('tenant', 'seat')),
    task_limit integer NOT NULL CHECK (task_limit > 0),
    task_limit_period text NOT NULL CHECK (task_limit_period IN ('total', 'month')),
    workspace_seconds_limit bigint NOT NULL CHECK (workspace_seconds_limit > 0),
    workspace_limit_period text NOT NULL CHECK (workspace_limit_period IN ('total', 'month')),
    member_limit integer NOT NULL CHECK (member_limit > 0),
    max_concurrency integer NOT NULL CHECK (max_concurrency > 0),
    retention_days integer NOT NULL CHECK (retention_days > 0),
    audit_retention_days integer NOT NULL CHECK (audit_retention_days > 0),
    max_task_turns integer NOT NULL CHECK (max_task_turns > 0),
    max_task_cost_usd numeric(10, 2) NOT NULL CHECK (max_task_cost_usd > 0),
    max_task_timeout_seconds bigint NOT NULL CHECK (max_task_timeout_seconds > 0),
    max_idle_seconds bigint NOT NULL CHECK (max_idle_seconds > 0),
    max_children_depth integer NOT NULL CHECK (max_children_depth >= 0),
    max_children_count integer NOT NULL CHECK (max_children_count >= 0),
    computes text[] NOT NULL CHECK (computes <@ ARRAY['standard', 'docker']::text[]),
    stripe_price_id text UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO sanchoris.plans (id, name, price_cents, currency, billing_unit, task_limit, task_limit_period, workspace_seconds_limit, workspace_limit_period, member_limit, max_concurrency, retention_days, audit_retention_days, max_task_turns, max_task_cost_usd, max_task_timeout_seconds, max_idle_seconds, max_children_depth, max_children_count, computes)
VALUES
    ('free', 'Free', 0, 'usd', 'tenant', 5, 'total', 36000, 'total', 1, 1, 30, 30, 100, 20, 86400, 1800, 1, 5, ARRAY['standard']),
    ('starter', 'Starter', 2000, 'usd', 'tenant', 10, 'month', 180000, 'month', 3, 2, 90, 365, 200, 50, 86400, 3600, 2, 10, ARRAY['standard', 'docker']),
    ('pro', 'Pro', 10000, 'usd', 'tenant', 50, 'month', 900000, 'month', 10, 5, 90, 365, 200, 100, 86400, 3600, 3, 20, ARRAY['standard', 'docker']),
    ('max', 'Max', 20000, 'usd', 'tenant', 200, 'month', 1800000, 'month', 30, 10, 90, 365, 400, 200, 172800, 7200, 3, 50, ARRAY['standard', 'docker'])
ON CONFLICT (id) DO NOTHING;

CREATE TABLE IF NOT EXISTS sanchoris.users (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    cognito_subject text NOT NULL UNIQUE,
    email text NOT NULL UNIQUE,
    display_name text NOT NULL,
    github_user_id bigint UNIQUE,
    github_login text UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((github_user_id IS NULL) = (github_login IS NULL))
);

CREATE TABLE IF NOT EXISTS sanchoris.tenants (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL,
    plan_id text NOT NULL REFERENCES sanchoris.plans(id),
    period_start timestamptz NOT NULL,
    tasks_this_period integer NOT NULL DEFAULT 0 CHECK (tasks_this_period >= 0),
    workspace_seconds_this_period bigint NOT NULL DEFAULT 0 CHECK (workspace_seconds_this_period >= 0),
    status text NOT NULL CHECK (status IN ('active', 'suspended')),
    suspended_reason text,
    stripe_customer_id text UNIQUE,
    created_by_user_id uuid NOT NULL REFERENCES sanchoris.users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((status = 'suspended') = (suspended_reason IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS sanchoris.members (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES sanchoris.users(id) ON DELETE CASCADE,
    role text NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    joined_at timestamptz NOT NULL DEFAULT now(),
    last_active_at timestamptz,
    UNIQUE (user_id, tenant_id)
);

CREATE TABLE IF NOT EXISTS sanchoris.invitations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    email text NOT NULL,
    role text NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    status text NOT NULL CHECK (status IN ('pending', 'accepted', 'revoked', 'expired')),
    invited_by_member_id uuid NOT NULL REFERENCES sanchoris.members(id) ON DELETE CASCADE,
    expires_at timestamptz NOT NULL,
    accepted_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((status = 'accepted') = (accepted_at IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS sanchoris.audit_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id),
    at timestamptz NOT NULL DEFAULT now(),
    actor_kind text NOT NULL CHECK (actor_kind IN ('member', 'agent', 'trigger', 'system', 'operator')),
    actor_ref text NOT NULL,
    action text NOT NULL,
    target_kind text NOT NULL,
    target_ref text NOT NULL,
    summary text NOT NULL,
    source_ip inet,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE TABLE IF NOT EXISTS sanchoris.credentials (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    scope text NOT NULL CHECK (scope IN ('personal', 'org')),
    member_id uuid REFERENCES sanchoris.members(id) ON DELETE CASCADE,
    name text NOT NULL,
    provider text NOT NULL CHECK (provider IN ('claude', 'codex')),
    kind text NOT NULL CHECK (kind IN ('subscription', 'api_key', 'bedrock')),
    account text NOT NULL,
    status text NOT NULL CHECK (status IN ('connected', 'reconnect')),
    encrypted_value bytea NOT NULL,
    kms_key_id text NOT NULL,
    last_used_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, name),
    CHECK ((scope = 'personal') = (member_id IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS sanchoris.usage_snapshots (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    credential_id uuid NOT NULL REFERENCES sanchoris.credentials(id) ON DELETE CASCADE,
    captured_at timestamptz NOT NULL,
    five_hour_utilization numeric(5, 2) NOT NULL,
    five_hour_resets_at timestamptz NOT NULL,
    seven_day_utilization numeric(5, 2) NOT NULL,
    seven_day_resets_at timestamptz NOT NULL
);

CREATE TABLE IF NOT EXISTS sanchoris.integrations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('github_installation', 'slack')),
    status text NOT NULL CHECK (status IN ('connected', 'not_connected')),
    github_installation_id bigint UNIQUE,
    slack_team_id text UNIQUE,
    details jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((kind = 'github_installation') = (github_installation_id IS NOT NULL)),
    CHECK ((kind = 'slack') = (slack_team_id IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS sanchoris.configs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    repository text NOT NULL,
    root_path text NOT NULL,
    active_revision_id uuid,
    synced_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, repository, root_path)
);

CREATE TABLE IF NOT EXISTS sanchoris.config_revisions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    config_id uuid NOT NULL REFERENCES sanchoris.configs(id) ON DELETE CASCADE,
    commit_sha text NOT NULL,
    parsed jsonb,
    validation_status text NOT NULL CHECK (validation_status IN ('valid', 'invalid')),
    validation_messages jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (config_id, commit_sha),
    CHECK (validation_status = 'invalid' OR parsed IS NOT NULL)
);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'configs_active_revision_id_fkey'
          AND conrelid = 'sanchoris.configs'::regclass
    ) THEN
        ALTER TABLE sanchoris.configs
            ADD CONSTRAINT configs_active_revision_id_fkey
            FOREIGN KEY (active_revision_id)
            REFERENCES sanchoris.config_revisions(id)
            DEFERRABLE INITIALLY DEFERRED;
    END IF;
END
$$;

CREATE TABLE IF NOT EXISTS sanchoris.projects (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    config_id uuid NOT NULL REFERENCES sanchoris.configs(id) ON DELETE CASCADE,
    config_revision_id uuid NOT NULL REFERENCES sanchoris.config_revisions(id),
    name text NOT NULL,
    system_prompt text NOT NULL,
    source_path text NOT NULL,
    UNIQUE (config_id, name)
);

CREATE TABLE IF NOT EXISTS sanchoris.repositories (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    project_id uuid NOT NULL REFERENCES sanchoris.projects(id) ON DELETE CASCADE,
    name text NOT NULL,
    url text NOT NULL,
    warm_hook jsonb,
    start_hook jsonb,
    prebuild_status text CHECK (prebuild_status IN ('pending', 'running', 'succeeded', 'failed')),
    prebuild_at timestamptz,
    UNIQUE (project_id, name)
);

CREATE TABLE IF NOT EXISTS sanchoris.sessions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    config_id uuid NOT NULL REFERENCES sanchoris.configs(id) ON DELETE CASCADE,
    config_revision_id uuid NOT NULL REFERENCES sanchoris.config_revisions(id),
    name text NOT NULL,
    definition jsonb NOT NULL,
    UNIQUE (config_id, name)
);

CREATE TABLE IF NOT EXISTS sanchoris.skills (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    config_id uuid NOT NULL REFERENCES sanchoris.configs(id) ON DELETE CASCADE,
    config_revision_id uuid NOT NULL REFERENCES sanchoris.config_revisions(id),
    name text NOT NULL,
    kind text NOT NULL CHECK (kind IN ('reliable', 'regular')),
    description text NOT NULL,
    skill_md text NOT NULL,
    skill_yaml text,
    definition jsonb,
    layout jsonb,
    UNIQUE (config_id, name),
    CHECK ((kind = 'reliable') = (skill_yaml IS NOT NULL)),
    CHECK ((skill_yaml IS NULL) = (definition IS NULL))
);

CREATE TABLE IF NOT EXISTS sanchoris.triggers (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    config_id uuid NOT NULL REFERENCES sanchoris.configs(id) ON DELETE CASCADE,
    config_revision_id uuid NOT NULL REFERENCES sanchoris.config_revisions(id),
    name text NOT NULL,
    state text NOT NULL CHECK (state IN ('ENABLED', 'DISABLED')),
    event_pattern jsonb,
    schedule_expression text,
    input_transformer jsonb,
    start jsonb NOT NULL,
    removed_at timestamptz,
    UNIQUE (config_id, name),
    CHECK ((event_pattern IS NULL) <> (schedule_expression IS NULL))
);

CREATE TABLE IF NOT EXISTS sanchoris.trigger_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    trigger_id uuid NOT NULL REFERENCES sanchoris.triggers(id) ON DELETE CASCADE,
    key text NOT NULL,
    event jsonb NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    result text CHECK (result IN ('started', 'skipped', 'discarded')),
    reason text,
    evaluated_at timestamptz,
    UNIQUE (trigger_id, key),
    CHECK ((result IS NULL) = (evaluated_at IS NULL)),
    CHECK ((result IN ('skipped', 'discarded')) = (reason IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS sanchoris.tasks (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    trigger_id uuid NOT NULL REFERENCES sanchoris.triggers(id),
    trigger_event_id uuid REFERENCES sanchoris.trigger_events(id) ON DELETE SET NULL,
    config_revision_id uuid NOT NULL REFERENCES sanchoris.config_revisions(id),
    name text NOT NULL,
    title text NOT NULL,
    status text NOT NULL CHECK (status IN ('PENDING', 'RUNNING', 'WAITING', 'NEEDS_YOU', 'SUCCEEDED', 'FAILED', 'ABORTED')),
    wait_reason text CHECK (wait_reason IN ('skill', 'usage_limit', 'chat')),
    resume_at timestamptz,
    nudges integer NOT NULL DEFAULT 0 CHECK (nudges >= 0),
    definition jsonb NOT NULL,
    parent_task_id uuid REFERENCES sanchoris.tasks(id) ON DELETE SET NULL,
    channel jsonb,
    start_payload jsonb,
    requested_by_member_id uuid REFERENCES sanchoris.members(id) ON DELETE SET NULL,
    error text,
    cause text,
    created_at timestamptz NOT NULL DEFAULT now(),
    started_at timestamptz,
    finished_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (trigger_id, name),
    CHECK ((status = 'WAITING') = (wait_reason IS NOT NULL)),
    CHECK (resume_at IS NULL OR wait_reason = 'usage_limit' OR status = 'NEEDS_YOU'),
    CHECK (wait_reason IS DISTINCT FROM 'usage_limit' OR resume_at IS NOT NULL),
    CHECK (error IS NULL OR status IN ('NEEDS_YOU', 'FAILED')),
    CHECK ((status IN ('SUCCEEDED', 'FAILED', 'ABORTED')) = (finished_at IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS sanchoris.task_dependencies (
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    task_id uuid NOT NULL REFERENCES sanchoris.tasks(id) ON DELETE CASCADE,
    blocked_by_task_id uuid NOT NULL REFERENCES sanchoris.tasks(id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, blocked_by_task_id),
    CHECK (task_id <> blocked_by_task_id)
);

CREATE TABLE IF NOT EXISTS sanchoris.change_requests (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    config_id uuid NOT NULL REFERENCES sanchoris.configs(id) ON DELETE CASCADE,
    pr_number integer NOT NULL,
    state text NOT NULL CHECK (state IN ('open', 'merged', 'closed')),
    changed_files jsonb NOT NULL DEFAULT '[]'::jsonb,
    changed_definitions jsonb NOT NULL DEFAULT '[]'::jsonb,
    validation_status text CHECK (validation_status IN ('valid', 'invalid')),
    validation_messages jsonb NOT NULL DEFAULT '[]'::jsonb,
    requested_by_member_id uuid REFERENCES sanchoris.members(id) ON DELETE SET NULL,
    requested_by_task_id uuid REFERENCES sanchoris.tasks(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (config_id, pr_number)
);

CREATE TABLE IF NOT EXISTS sanchoris.runs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    task_id uuid NOT NULL REFERENCES sanchoris.tasks(id) ON DELETE CASCADE,
    session_name text NOT NULL,
    worker text NOT NULL CHECK (worker IN ('claude', 'codex')),
    model text NOT NULL,
    agent_session_id text,
    status text NOT NULL CHECK (status IN ('creating', 'running', 'idle', 'completed', 'failed')),
    turns integer NOT NULL DEFAULT 0 CHECK (turns >= 0),
    cost_usd numeric(12, 6) NOT NULL DEFAULT 0 CHECK (cost_usd >= 0),
    summary text,
    error text,
    resume_branch text,
    record_s3_key text,
    log_s3_key text,
    last_heartbeat_at timestamptz,
    started_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sanchoris.workspaces (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    run_id uuid NOT NULL REFERENCES sanchoris.runs(id) ON DELETE CASCADE,
    compute text NOT NULL CHECK (compute IN ('standard', 'docker')),
    platform text NOT NULL CHECK (platform IN ('fargate', 'lambda_microvm', 'local')),
    platform_ref text,
    repositories jsonb NOT NULL DEFAULT '[]'::jsonb,
    branch text NOT NULL,
    base_commit text,
    started_at timestamptz NOT NULL DEFAULT now(),
    stopped_at timestamptz,
    CHECK (stopped_at IS NULL OR stopped_at >= started_at)
);

CREATE TABLE IF NOT EXISTS sanchoris.skill_runs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    task_id uuid NOT NULL REFERENCES sanchoris.tasks(id) ON DELETE CASCADE,
    run_id uuid REFERENCES sanchoris.runs(id) ON DELETE SET NULL,
    number integer NOT NULL CHECK (number > 0),
    skill_name text NOT NULL,
    definition jsonb NOT NULL,
    input jsonb NOT NULL,
    output jsonb,
    status text NOT NULL CHECK (status IN ('RUNNING', 'WAITING_AGENT', 'WAITING', 'SUCCEEDED', 'FAILED')),
    current_state text,
    state jsonb NOT NULL,
    error text,
    cause text,
    started_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (task_id, number),
    CHECK ((status IN ('SUCCEEDED', 'FAILED')) = (finished_at IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS sanchoris.state_entries (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    skill_run_id uuid NOT NULL REFERENCES sanchoris.skill_runs(id) ON DELETE CASCADE,
    sequence integer NOT NULL CHECK (sequence >= 0),
    state_name text NOT NULL,
    state_type text NOT NULL CHECK (state_type IN ('Task', 'Pass', 'Choice', 'Wait', 'Succeed', 'Fail')),
    status text NOT NULL CHECK (status IN ('RUNNING', 'WAITING', 'SUCCEEDED', 'FAILED', 'CAUGHT', 'TIMED_OUT')),
    entered_at timestamptz NOT NULL DEFAULT now(),
    exited_at timestamptz,
    input jsonb NOT NULL,
    output jsonb,
    error text,
    cause text,
    attempts integer NOT NULL DEFAULT 1 CHECK (attempts > 0),
    next_state text,
    UNIQUE (skill_run_id, sequence)
);

CREATE TABLE IF NOT EXISTS sanchoris.outputs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    skill_run_id uuid NOT NULL REFERENCES sanchoris.skill_runs(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('pull_request', 'comment')),
    repository text NOT NULL,
    number integer NOT NULL,
    url text NOT NULL,
    state text CHECK (state IN ('open', 'merged', 'closed')),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((kind = 'pull_request') = (state IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS sanchoris.usage (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    run_id uuid NOT NULL REFERENCES sanchoris.runs(id) ON DELETE CASCADE,
    input_tokens bigint NOT NULL CHECK (input_tokens >= 0),
    output_tokens bigint NOT NULL CHECK (output_tokens >= 0),
    cache_read_tokens bigint NOT NULL CHECK (cache_read_tokens >= 0),
    cache_write_tokens bigint NOT NULL CHECK (cache_write_tokens >= 0),
    cost_usd numeric(12, 6) NOT NULL CHECK (cost_usd >= 0),
    recorded_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sanchoris.due_jobs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind <> ''),
    ref_id uuid NOT NULL,
    due_at timestamptz NOT NULL,
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    payload jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (kind, ref_id)
);

CREATE TABLE IF NOT EXISTS sanchoris.inbound_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    source text NOT NULL CHECK (source IN ('github', 'slack', 'web', 'schedule')),
    key text NOT NULL,
    event jsonb NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    processed_at timestamptz,
    UNIQUE (tenant_id, source, key)
);

CREATE TABLE IF NOT EXISTS sanchoris.run_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    run_id uuid NOT NULL REFERENCES sanchoris.runs(id) ON DELETE CASCADE,
    seq bigint,
    type text NOT NULL CHECK (type IN ('ready', 'turn_end', 'rate_limit', 'heartbeat', 'session_record', 'exec_result', 'human_reply')),
    payload jsonb NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    processed_at timestamptz,
    error text,
    UNIQUE (run_id, seq)
);

CREATE TABLE IF NOT EXISTS sanchoris.run_outbox (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    run_id uuid NOT NULL REFERENCES sanchoris.runs(id) ON DELETE CASCADE,
    seq bigint GENERATED ALWAYS AS IDENTITY,
    type text NOT NULL CHECK (type IN ('start', 'user_message', 'exec', 'interrupt', 'shutdown', 'chat_post')),
    payload jsonb NOT NULL,
    dedupe_key text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    delivered_at timestamptz,
    UNIQUE (run_id, dedupe_key)
);

CREATE TABLE IF NOT EXISTS sanchoris.resource_invocations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    skill_run_id uuid NOT NULL REFERENCES sanchoris.skill_runs(id) ON DELETE CASCADE,
    idempotency_key text NOT NULL,
    resource text NOT NULL,
    invocation jsonb NOT NULL,
    status text NOT NULL CHECK (status IN ('pending', 'awaiting', 'done')),
    result jsonb,
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    UNIQUE (skill_run_id, idempotency_key),
    CHECK ((status = 'done') = (completed_at IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS sanchoris.check_waits (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    skill_run_id uuid NOT NULL REFERENCES sanchoris.skill_runs(id) ON DELETE CASCADE,
    task_token text NOT NULL,
    repos jsonb NOT NULL,
    head_shas jsonb NOT NULL DEFAULT '{}'::jsonb,
    status text NOT NULL CHECK (status IN ('waiting', 'done')),
    next_poll_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    UNIQUE (skill_run_id, task_token),
    CHECK ((status = 'done') = (completed_at IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS sanchoris.approval_waits (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    task_id uuid NOT NULL REFERENCES sanchoris.tasks(id) ON DELETE CASCADE,
    skill_run_id uuid NOT NULL REFERENCES sanchoris.skill_runs(id) ON DELETE CASCADE,
    task_token text NOT NULL,
    message text NOT NULL,
    status text NOT NULL CHECK (status IN ('waiting', 'approved', 'rejected')),
    created_at timestamptz NOT NULL DEFAULT now(),
    resolved_at timestamptz,
    UNIQUE (skill_run_id, task_token),
    CHECK ((status = 'waiting') = (resolved_at IS NULL))
);

CREATE TABLE IF NOT EXISTS sanchoris.usage_alerts (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    period_start timestamptz NOT NULL,
    metric text NOT NULL CHECK (metric IN ('tasks', 'workspace_seconds')),
    threshold_percent integer NOT NULL CHECK (threshold_percent > 0),
    used bigint NOT NULL,
    limit_value bigint NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, period_start, metric, threshold_percent)
);

CREATE TABLE IF NOT EXISTS sanchoris.run_channel_instances (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    run_id uuid NOT NULL REFERENCES sanchoris.runs(id) ON DELETE CASCADE,
    instance_id uuid NOT NULL,
    seq_offset bigint NOT NULL CHECK (seq_offset >= 0),
    processed_seq bigint NOT NULL DEFAULT 0 CHECK (processed_seq >= 0),
    started boolean NOT NULL DEFAULT false,
    connected_at timestamptz NOT NULL,
    UNIQUE (run_id, instance_id),
    UNIQUE (run_id, seq_offset)
);

CREATE TABLE IF NOT EXISTS sanchoris.run_log_entries (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    run_id uuid NOT NULL REFERENCES sanchoris.runs(id) ON DELETE CASCADE,
    seq bigint NOT NULL,
    turn integer NOT NULL CHECK (turn > 0),
    kind text NOT NULL CHECK (kind IN ('text', 'tool_use', 'tool_result', 'item', 'system', 'log')),
    data jsonb NOT NULL,
    at timestamptz NOT NULL,
    UNIQUE (run_id, seq)
);

CREATE TABLE IF NOT EXISTS sanchoris.chat_messages (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES sanchoris.tenants(id) ON DELETE CASCADE,
    task_id uuid NOT NULL REFERENCES sanchoris.tasks(id) ON DELETE CASCADE,
    run_id uuid REFERENCES sanchoris.runs(id) ON DELETE SET NULL,
    outbox_id uuid UNIQUE,
    channel jsonb,
    author_kind text NOT NULL CHECK (author_kind IN ('agent', 'member')),
    member_id uuid REFERENCES sanchoris.members(id) ON DELETE SET NULL,
    text text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((author_kind = 'member') = (member_id IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS members_tenant_idx ON sanchoris.members (tenant_id);
CREATE UNIQUE INDEX IF NOT EXISTS invitations_pending_email_idx ON sanchoris.invitations (tenant_id, email) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS audit_events_tenant_at_idx ON sanchoris.audit_events (tenant_id, at);
CREATE INDEX IF NOT EXISTS usage_snapshots_credential_captured_idx ON sanchoris.usage_snapshots (credential_id, captured_at DESC);
CREATE INDEX IF NOT EXISTS config_revisions_config_created_idx ON sanchoris.config_revisions (config_id, created_at DESC);
CREATE INDEX IF NOT EXISTS triggers_tenant_idx ON sanchoris.triggers (tenant_id);
CREATE INDEX IF NOT EXISTS trigger_events_trigger_key_idx ON sanchoris.trigger_events (trigger_id, key);
CREATE INDEX IF NOT EXISTS trigger_events_trigger_received_idx ON sanchoris.trigger_events (trigger_id, received_at DESC);
CREATE INDEX IF NOT EXISTS trigger_events_unevaluated_idx ON sanchoris.trigger_events (received_at) WHERE result IS NULL;
CREATE INDEX IF NOT EXISTS tasks_tenant_status_idx ON sanchoris.tasks (tenant_id, status);
CREATE INDEX IF NOT EXISTS tasks_trigger_name_idx ON sanchoris.tasks (trigger_id, name);
CREATE INDEX IF NOT EXISTS tasks_parent_idx ON sanchoris.tasks (parent_task_id);
CREATE INDEX IF NOT EXISTS task_dependencies_blocked_by_idx ON sanchoris.task_dependencies (blocked_by_task_id);
CREATE INDEX IF NOT EXISTS runs_task_idx ON sanchoris.runs (task_id, started_at DESC);
CREATE INDEX IF NOT EXISTS workspaces_run_idx ON sanchoris.workspaces (run_id);
CREATE INDEX IF NOT EXISTS skill_runs_task_idx ON sanchoris.skill_runs (task_id, number);
CREATE INDEX IF NOT EXISTS state_entries_skill_run_sequence_idx ON sanchoris.state_entries (skill_run_id, sequence);
CREATE INDEX IF NOT EXISTS outputs_skill_run_idx ON sanchoris.outputs (skill_run_id);
CREATE INDEX IF NOT EXISTS usage_run_idx ON sanchoris.usage (run_id);
CREATE INDEX IF NOT EXISTS due_jobs_due_at_idx ON sanchoris.due_jobs (due_at);
CREATE INDEX IF NOT EXISTS inbound_events_unprocessed_idx ON sanchoris.inbound_events (received_at) WHERE processed_at IS NULL;
CREATE INDEX IF NOT EXISTS run_events_unprocessed_idx ON sanchoris.run_events (received_at) WHERE processed_at IS NULL;
CREATE INDEX IF NOT EXISTS run_outbox_undelivered_idx ON sanchoris.run_outbox (run_id, seq) WHERE delivered_at IS NULL;
CREATE INDEX IF NOT EXISTS resource_invocations_pending_idx ON sanchoris.resource_invocations (created_at) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS check_waits_waiting_idx ON sanchoris.check_waits (next_poll_at) WHERE status = 'waiting';
CREATE INDEX IF NOT EXISTS chat_messages_task_created_idx ON sanchoris.chat_messages (task_id, created_at);

CREATE OR REPLACE TRIGGER audit_events_reject_update_delete
    BEFORE UPDATE OR DELETE ON sanchoris.audit_events
    FOR EACH ROW EXECUTE FUNCTION sanchoris.reject_audit_event_change();

CREATE OR REPLACE TRIGGER audit_events_reject_truncate
    BEFORE TRUNCATE ON sanchoris.audit_events
    FOR EACH STATEMENT EXECUTE FUNCTION sanchoris.reject_audit_event_change();

DO $$
DECLARE
    table_name text;
BEGIN
    FOREACH table_name IN ARRAY ARRAY[
        'members', 'invitations', 'audit_events', 'credentials', 'usage_snapshots',
        'integrations', 'configs', 'config_revisions', 'projects', 'repositories',
        'sessions', 'skills', 'triggers', 'trigger_events', 'tasks', 'task_dependencies',
        'change_requests', 'runs', 'workspaces', 'skill_runs', 'state_entries',
        'outputs', 'usage', 'due_jobs', 'inbound_events', 'run_events', 'run_outbox',
        'resource_invocations', 'check_waits', 'approval_waits', 'usage_alerts',
        'run_channel_instances', 'run_log_entries', 'chat_messages'
    ]
    LOOP
        EXECUTE format('ALTER TABLE sanchoris.%I ENABLE ROW LEVEL SECURITY', table_name);
        EXECUTE format('ALTER TABLE sanchoris.%I FORCE ROW LEVEL SECURITY', table_name);
        EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON sanchoris.%I', table_name);
        EXECUTE format(
            'CREATE POLICY tenant_isolation ON sanchoris.%I TO sanchoris_app USING (tenant_id = sanchoris.current_tenant_id()) WITH CHECK (tenant_id = sanchoris.current_tenant_id())',
            table_name
        );
        EXECUTE format('DROP POLICY IF EXISTS worker_access ON sanchoris.%I', table_name);
        EXECUTE format(
            'CREATE POLICY worker_access ON sanchoris.%I TO sanchoris_worker USING (true) WITH CHECK (true)',
            table_name
        );
    END LOOP;
END
$$;

ALTER TABLE sanchoris.tenants ENABLE ROW LEVEL SECURITY;
ALTER TABLE sanchoris.tenants FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON sanchoris.tenants;
CREATE POLICY tenant_isolation ON sanchoris.tenants TO sanchoris_app
    USING (id = sanchoris.current_tenant_id())
    WITH CHECK (id = sanchoris.current_tenant_id());
DROP POLICY IF EXISTS worker_access ON sanchoris.tenants;
CREATE POLICY worker_access ON sanchoris.tenants TO sanchoris_worker USING (true) WITH CHECK (true);

GRANT USAGE ON SCHEMA sanchoris TO sanchoris_app, sanchoris_worker;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA sanchoris TO sanchoris_app, sanchoris_worker;
REVOKE INSERT, UPDATE, DELETE ON sanchoris.plans FROM sanchoris_app, sanchoris_worker;
REVOKE UPDATE, DELETE, TRUNCATE ON sanchoris.audit_events FROM sanchoris_app, sanchoris_worker;
GRANT EXECUTE ON FUNCTION sanchoris.current_tenant_id() TO sanchoris_app, sanchoris_worker;
