CREATE SCHEMA IF NOT EXISTS sanchoris;

CREATE TABLE IF NOT EXISTS sanchoris.users (
    id uuid PRIMARY KEY,
    cognito_subject text NOT NULL UNIQUE,
    email text NOT NULL UNIQUE,
    display_name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sanchoris.project_profiles (
    id uuid PRIMARY KEY,
    name text NOT NULL,
    repository_path text NOT NULL,
    default_branch text NOT NULL,
    worktree_policy text NOT NULL,
    dev_command text NOT NULL,
    check_command text NOT NULL,
    worker_policy text NOT NULL,
    allowed_workflow_id uuid,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sanchoris.conversations (
    id uuid PRIMARY KEY,
    project_profile_id uuid NOT NULL REFERENCES sanchoris.project_profiles(id) ON DELETE CASCADE,
    title text NOT NULL,
    channel text NOT NULL,
    created_by_user_id uuid REFERENCES sanchoris.users(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sanchoris.chat_messages (
    id uuid PRIMARY KEY,
    conversation_id uuid NOT NULL REFERENCES sanchoris.conversations(id) ON DELETE CASCADE,
    author_kind text NOT NULL CHECK (author_kind IN ('user', 'sanchoris', 'worker', 'system')),
    author_user_id uuid REFERENCES sanchoris.users(id) ON DELETE SET NULL,
    body text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sanchoris.workflow_specs (
    id uuid PRIMARY KEY,
    project_profile_id uuid NOT NULL REFERENCES sanchoris.project_profiles(id) ON DELETE CASCADE,
    name text NOT NULL,
    version text NOT NULL,
    yaml text NOT NULL,
    created_by_user_id uuid REFERENCES sanchoris.users(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_profile_id, name, version)
);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'project_profiles_allowed_workflow_id_fkey'
          AND conrelid = 'sanchoris.project_profiles'::regclass
    ) THEN
        ALTER TABLE sanchoris.project_profiles
            ADD CONSTRAINT project_profiles_allowed_workflow_id_fkey
            FOREIGN KEY (allowed_workflow_id)
            REFERENCES sanchoris.workflow_specs(id)
            DEFERRABLE INITIALLY DEFERRED;
    END IF;
END
$$;

CREATE TABLE IF NOT EXISTS sanchoris.workflow_blocks (
    id uuid PRIMARY KEY,
    workflow_spec_id uuid NOT NULL REFERENCES sanchoris.workflow_specs(id) ON DELETE CASCADE,
    kind text NOT NULL,
    label text NOT NULL,
    state text NOT NULL CHECK (state IN ('pending', 'running', 'passed', 'failed', 'blocked')),
    position_x integer NOT NULL,
    position_y integer NOT NULL,
    config jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sanchoris.workflow_edges (
    id uuid PRIMARY KEY,
    workflow_spec_id uuid NOT NULL REFERENCES sanchoris.workflow_specs(id) ON DELETE CASCADE,
    from_block_id uuid NOT NULL REFERENCES sanchoris.workflow_blocks(id) ON DELETE CASCADE,
    to_block_id uuid NOT NULL REFERENCES sanchoris.workflow_blocks(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workflow_spec_id, from_block_id, to_block_id)
);

CREATE TABLE IF NOT EXISTS sanchoris.tasks (
    id uuid PRIMARY KEY,
    project_profile_id uuid NOT NULL REFERENCES sanchoris.project_profiles(id) ON DELETE CASCADE,
    conversation_id uuid REFERENCES sanchoris.conversations(id) ON DELETE SET NULL,
    source_message_id uuid REFERENCES sanchoris.chat_messages(id) ON DELETE SET NULL,
    assigned_workflow_id uuid NOT NULL REFERENCES sanchoris.workflow_specs(id),
    title text NOT NULL,
    description text NOT NULL,
    status text NOT NULL CHECK (status IN ('queued', 'running', 'review', 'done', 'failed')),
    priority text NOT NULL CHECK (priority IN ('low', 'medium', 'high')),
    assigned_worker text NOT NULL,
    review_state text NOT NULL,
    created_by_user_id uuid REFERENCES sanchoris.users(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sanchoris.workspaces (
    id uuid PRIMARY KEY,
    task_id uuid NOT NULL REFERENCES sanchoris.tasks(id) ON DELETE CASCADE,
    branch text NOT NULL,
    worktree_path text NOT NULL,
    base_commit text NOT NULL,
    current_commit text,
    changed_files jsonb NOT NULL DEFAULT '[]'::jsonb,
    verification_commands jsonb NOT NULL DEFAULT '[]'::jsonb,
    cleanup_state text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (task_id, branch)
);

CREATE TABLE IF NOT EXISTS sanchoris.workflow_runs (
    id uuid PRIMARY KEY,
    task_id uuid NOT NULL REFERENCES sanchoris.tasks(id) ON DELETE CASCADE,
    workspace_id uuid REFERENCES sanchoris.workspaces(id) ON DELETE SET NULL,
    worker_kind text NOT NULL,
    prompt_summary text NOT NULL,
    status text NOT NULL CHECK (status IN ('queued', 'running', 'gate_pending', 'passed', 'failed')),
    started_at timestamptz,
    finished_at timestamptz,
    exit_code integer,
    commit_hash text,
    log_uri text NOT NULL,
    error_summary text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sanchoris.workflow_step_runs (
    id uuid PRIMARY KEY,
    workflow_run_id uuid NOT NULL REFERENCES sanchoris.workflow_runs(id) ON DELETE CASCADE,
    workflow_block_id uuid REFERENCES sanchoris.workflow_blocks(id) ON DELETE SET NULL,
    status text NOT NULL CHECK (status IN ('pending', 'running', 'passed', 'failed', 'blocked')),
    started_at timestamptz,
    finished_at timestamptz,
    summary text NOT NULL DEFAULT '',
    artifact_uri text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sanchoris.verification_results (
    id uuid PRIMARY KEY,
    workflow_run_id uuid NOT NULL REFERENCES sanchoris.workflow_runs(id) ON DELETE CASCADE,
    command text NOT NULL,
    exit_code integer,
    status text NOT NULL CHECK (status IN ('pending', 'running', 'passed', 'failed', 'blocked')),
    summary text NOT NULL,
    artifact_uri text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sanchoris.reviews (
    id uuid PRIMARY KEY,
    workflow_run_id uuid NOT NULL REFERENCES sanchoris.workflow_runs(id) ON DELETE CASCADE,
    workflow_step_run_id uuid REFERENCES sanchoris.workflow_step_runs(id) ON DELETE SET NULL,
    state text NOT NULL CHECK (state IN ('pending', 'approved', 'rejected', 'cancelled')),
    review_target text NOT NULL,
    approver_user_id uuid REFERENCES sanchoris.users(id) ON DELETE SET NULL,
    decided_at timestamptz,
    decision_reason text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sanchoris.pull_requests (
    id uuid PRIMARY KEY,
    workflow_run_id uuid NOT NULL REFERENCES sanchoris.workflow_runs(id) ON DELETE CASCADE,
    url text,
    source_branch text NOT NULL,
    base_branch text NOT NULL,
    commit_hash text NOT NULL,
    status text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sanchoris.merges (
    id uuid PRIMARY KEY,
    pull_request_id uuid NOT NULL REFERENCES sanchoris.pull_requests(id) ON DELETE CASCADE,
    method text NOT NULL,
    status text NOT NULL,
    merge_commit text,
    merged_at timestamptz,
    merged_by_user_id uuid REFERENCES sanchoris.users(id) ON DELETE SET NULL,
    failure_reason text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS sanchoris.events (
    id uuid PRIMARY KEY,
    project_profile_id uuid REFERENCES sanchoris.project_profiles(id) ON DELETE CASCADE,
    task_id uuid REFERENCES sanchoris.tasks(id) ON DELETE CASCADE,
    workflow_run_id uuid REFERENCES sanchoris.workflow_runs(id) ON DELETE CASCADE,
    actor_user_id uuid REFERENCES sanchoris.users(id) ON DELETE SET NULL,
    event_type text NOT NULL,
    body text NOT NULL,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS chat_messages_conversation_created_idx ON sanchoris.chat_messages(conversation_id, created_at);
CREATE INDEX IF NOT EXISTS tasks_project_status_idx ON sanchoris.tasks(project_profile_id, status);
CREATE INDEX IF NOT EXISTS workflow_runs_task_created_idx ON sanchoris.workflow_runs(task_id, created_at DESC);
CREATE INDEX IF NOT EXISTS events_task_created_idx ON sanchoris.events(task_id, created_at DESC);
CREATE INDEX IF NOT EXISTS events_run_created_idx ON sanchoris.events(workflow_run_id, created_at DESC);
