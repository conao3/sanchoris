use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

const JSON_CONTENT_TYPE: &str = "application/json";

fn main() -> std::io::Result<()> {
    let address = backend_address();
    let listener = TcpListener::bind(&address)?;

    println!("sanchoris-backend listening on http://{address}");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => handle_connection(stream)?,
            Err(error) => eprintln!("failed to accept connection: {error}"),
        }
    }

    Ok(())
}

fn backend_address() -> String {
    std::env::var("SANCHORIS_BACKEND_ADDR").unwrap_or_else(|_| {
        std::env::var("PORT")
            .map(|port| format!("127.0.0.1:{port}"))
            .unwrap_or_else(|_| "127.0.0.1:3000".to_string())
    })
}

fn handle_connection(mut stream: TcpStream) -> std::io::Result<()> {
    let mut buffer = [0; 1024];
    let bytes_read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    let (status, content_type, body) = match path {
        "/health" | "/api/v1/health" => (
            "200 OK",
            JSON_CONTENT_TYPE,
            r#"{"status":"ok","service":"sanchoris-backend"}"#.to_string(),
        ),
        "/api/v1/mvp/project" => ("200 OK", JSON_CONTENT_TYPE, sample_project().to_json()),
        "/api/v1/mvp/tasks" => ("200 OK", JSON_CONTENT_TYPE, sample_tasks().to_json()),
        "/api/v1/mvp/workflow" => ("200 OK", JSON_CONTENT_TYPE, sample_workflow().to_json()),
        "/api/v1/mvp/runs" => ("200 OK", JSON_CONTENT_TYPE, sample_runs().to_json()),
        _ => (
            "200 OK",
            "text/plain; charset=utf-8",
            "sanchoris-backend is running".to_string(),
        ),
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())
}

#[derive(Clone, Copy)]
struct ProjectProfile {
    id: &'static str,
    name: &'static str,
    repository_path: &'static str,
    default_branch: &'static str,
    worktree_policy: &'static str,
    dev_command: &'static str,
    check_command: &'static str,
    worker_policy: &'static str,
    allowed_workflow: &'static str,
}

impl ProjectProfile {
    fn to_json(self) -> String {
        format!(
            "{{\"project\":{{\"id\":{},\"name\":{},\"repositoryPath\":{},\"defaultBranch\":{},\"worktreePolicy\":{},\"devCommand\":{},\"checkCommand\":{},\"workerPolicy\":{},\"allowedWorkflow\":{}}}}}",
            json_string(self.id),
            json_string(self.name),
            json_string(self.repository_path),
            json_string(self.default_branch),
            json_string(self.worktree_policy),
            json_string(self.dev_command),
            json_string(self.check_command),
            json_string(self.worker_policy),
            json_string(self.allowed_workflow),
        )
    }
}

#[derive(Clone, Copy)]
struct NativeTask {
    id: &'static str,
    conversation_id: &'static str,
    source_message_id: &'static str,
    title: &'static str,
    description: &'static str,
    status: &'static str,
    priority: &'static str,
    project_id: &'static str,
    assigned_workflow_id: &'static str,
    assigned_worker: &'static str,
    workspace_id: &'static str,
    latest_run_id: &'static str,
    review_state: &'static str,
}

impl NativeTask {
    fn to_json(self) -> String {
        format!(
            "{{\"id\":{},\"conversationId\":{},\"sourceMessageId\":{},\"source\":{},\"title\":{},\"description\":{},\"status\":{},\"priority\":{},\"projectId\":{},\"assignedWorkflowId\":{},\"assignedWorker\":{},\"workspaceId\":{},\"latestRunId\":{},\"reviewState\":{}}}",
            json_string(self.id),
            json_string(self.conversation_id),
            json_string(self.source_message_id),
            json_string("built-in-chat"),
            json_string(self.title),
            json_string(self.description),
            json_string(self.status),
            json_string(self.priority),
            json_string(self.project_id),
            json_string(self.assigned_workflow_id),
            json_string(self.assigned_worker),
            json_string(self.workspace_id),
            json_string(self.latest_run_id),
            json_string(self.review_state),
        )
    }
}

struct TaskSnapshot {
    conversation: Conversation,
    messages: Vec<ChatMessage>,
    tasks: Vec<NativeTask>,
}

impl TaskSnapshot {
    fn to_json(&self) -> String {
        format!(
            "{{\"conversation\":{},\"messages\":{},\"tasks\":{}}}",
            self.conversation.to_json(),
            json_array(self.messages.iter().map(|message| message.to_json())),
            json_array(self.tasks.iter().map(|task| task.to_json())),
        )
    }
}

#[derive(Clone, Copy)]
struct Conversation {
    id: &'static str,
    title: &'static str,
    channel: &'static str,
}

impl Conversation {
    fn to_json(self) -> String {
        format!(
            "{{\"id\":{},\"title\":{},\"channel\":{}}}",
            json_string(self.id),
            json_string(self.title),
            json_string(self.channel),
        )
    }
}

#[derive(Clone, Copy)]
struct ChatMessage {
    id: &'static str,
    conversation_id: &'static str,
    author: &'static str,
    body: &'static str,
    created_at: &'static str,
    task_id: Option<&'static str>,
}

impl ChatMessage {
    fn to_json(self) -> String {
        format!(
            "{{\"id\":{},\"conversationId\":{},\"author\":{},\"body\":{},\"createdAt\":{},\"taskId\":{}}}",
            json_string(self.id),
            json_string(self.conversation_id),
            json_string(self.author),
            json_string(self.body),
            json_string(self.created_at),
            json_optional_string(self.task_id),
        )
    }
}

struct WorkflowSpec {
    id: &'static str,
    name: &'static str,
    version: &'static str,
    format: &'static str,
    yaml: &'static str,
    blocks: Vec<WorkflowBlock>,
    edges: Vec<WorkflowEdge>,
}

impl WorkflowSpec {
    fn to_json(&self) -> String {
        format!(
            "{{\"workflow\":{{\"id\":{},\"name\":{},\"version\":{},\"format\":{},\"yaml\":{},\"blocks\":{},\"edges\":{}}}}}",
            json_string(self.id),
            json_string(self.name),
            json_string(self.version),
            json_string(self.format),
            json_string(self.yaml),
            json_array(self.blocks.iter().map(|block| block.to_json())),
            json_array(self.edges.iter().map(|edge| edge.to_json())),
        )
    }
}

#[derive(Clone, Copy)]
struct WorkflowBlock {
    id: &'static str,
    kind: &'static str,
    label: &'static str,
    state: &'static str,
    x: i32,
    y: i32,
}

impl WorkflowBlock {
    fn to_json(self) -> String {
        format!(
            "{{\"id\":{},\"kind\":{},\"label\":{},\"state\":{},\"position\":{{\"x\":{},\"y\":{}}}}}",
            json_string(self.id),
            json_string(self.kind),
            json_string(self.label),
            json_string(self.state),
            self.x,
            self.y,
        )
    }
}

#[derive(Clone, Copy)]
struct WorkflowEdge {
    from: &'static str,
    to: &'static str,
}

impl WorkflowEdge {
    fn to_json(self) -> String {
        format!(
            "{{\"from\":{},\"to\":{}}}",
            json_string(self.from),
            json_string(self.to),
        )
    }
}

struct RunSnapshot {
    runs: Vec<WorkerRun>,
}

impl RunSnapshot {
    fn to_json(&self) -> String {
        format!(
            "{{\"runs\":{}}}",
            json_array(self.runs.iter().map(|run| run.to_json()))
        )
    }
}

struct WorkerRun {
    id: &'static str,
    task_id: &'static str,
    worker_kind: &'static str,
    status: &'static str,
    started_at: &'static str,
    finished_at: Option<&'static str>,
    exit_code: Option<i32>,
    commit_hash: Option<&'static str>,
    log_path: &'static str,
    error_summary: Option<&'static str>,
    workspace: Workspace,
    verification: VerificationResult,
    pull_request: PullRequest,
    merge: MergeResult,
    gate: GateState,
}

impl WorkerRun {
    fn to_json(&self) -> String {
        format!(
            "{{\"id\":{},\"taskId\":{},\"workerKind\":{},\"status\":{},\"promptSummary\":{},\"startedAt\":{},\"finishedAt\":{},\"exitCode\":{},\"commitHash\":{},\"logPath\":{},\"errorSummary\":{},\"workspace\":{},\"verification\":{},\"pullRequest\":{},\"merge\":{},\"gate\":{}}}",
            json_string(self.id),
            json_string(self.task_id),
            json_string(self.worker_kind),
            json_string(self.status),
            json_string(
                "Implement the native task in an isolated worktree and return a committed change."
            ),
            json_string(self.started_at),
            json_optional_string(self.finished_at),
            json_optional_i32(self.exit_code),
            json_optional_string(self.commit_hash),
            json_string(self.log_path),
            json_optional_string(self.error_summary),
            self.workspace.to_json(),
            self.verification.to_json(),
            self.pull_request.to_json(),
            self.merge.to_json(),
            self.gate.to_json(),
        )
    }
}

#[derive(Clone, Copy)]
struct Workspace {
    id: &'static str,
    branch: &'static str,
    worktree_path: &'static str,
    base_commit: &'static str,
    current_commit: Option<&'static str>,
    cleanup_state: &'static str,
    changed_files: &'static [&'static str],
    verification_commands: &'static [&'static str],
}

impl Workspace {
    fn to_json(self) -> String {
        format!(
            "{{\"id\":{},\"branch\":{},\"worktreePath\":{},\"baseCommit\":{},\"currentCommit\":{},\"changedFiles\":{},\"verificationCommands\":{},\"cleanupState\":{}}}",
            json_string(self.id),
            json_string(self.branch),
            json_string(self.worktree_path),
            json_string(self.base_commit),
            json_optional_string(self.current_commit),
            json_array(self.changed_files.iter().map(|value| json_string(value))),
            json_array(
                self.verification_commands
                    .iter()
                    .map(|value| json_string(value))
            ),
            json_string(self.cleanup_state),
        )
    }
}

#[derive(Clone, Copy)]
struct VerificationResult {
    status: &'static str,
    command: &'static str,
    exit_code: Option<i32>,
    summary: &'static str,
    artifact_uri: &'static str,
}

impl VerificationResult {
    fn to_json(self) -> String {
        format!(
            "{{\"status\":{},\"command\":{},\"exitCode\":{},\"summary\":{},\"artifactUri\":{}}}",
            json_string(self.status),
            json_string(self.command),
            json_optional_i32(self.exit_code),
            json_string(self.summary),
            json_string(self.artifact_uri),
        )
    }
}

#[derive(Clone, Copy)]
struct PullRequest {
    url: Option<&'static str>,
    source_branch: &'static str,
    base_branch: &'static str,
    status: &'static str,
}

impl PullRequest {
    fn to_json(self) -> String {
        format!(
            "{{\"url\":{},\"sourceBranch\":{},\"baseBranch\":{},\"status\":{}}}",
            json_optional_string(self.url),
            json_string(self.source_branch),
            json_string(self.base_branch),
            json_string(self.status),
        )
    }
}

#[derive(Clone, Copy)]
struct MergeResult {
    method: &'static str,
    status: &'static str,
    merge_commit: Option<&'static str>,
    merged_at: Option<&'static str>,
    merged_by: Option<&'static str>,
    failure_reason: Option<&'static str>,
}

impl MergeResult {
    fn to_json(self) -> String {
        format!(
            "{{\"method\":{},\"status\":{},\"mergeCommit\":{},\"mergedAt\":{},\"mergedBy\":{},\"failureReason\":{}}}",
            json_string(self.method),
            json_string(self.status),
            json_optional_string(self.merge_commit),
            json_optional_string(self.merged_at),
            json_optional_string(self.merged_by),
            json_optional_string(self.failure_reason),
        )
    }
}

#[derive(Clone, Copy)]
struct GateState {
    id: &'static str,
    state: &'static str,
    review_target: &'static str,
    approver: Option<&'static str>,
    decided_at: Option<&'static str>,
}

impl GateState {
    fn to_json(self) -> String {
        format!(
            "{{\"id\":{},\"state\":{},\"reviewTarget\":{},\"approver\":{},\"decidedAt\":{}}}",
            json_string(self.id),
            json_string(self.state),
            json_string(self.review_target),
            json_optional_string(self.approver),
            json_optional_string(self.decided_at),
        )
    }
}

fn sample_project() -> ProjectProfile {
    ProjectProfile {
        id: "project_sanchoris",
        name: "Sanchoris",
        repository_path: "/home/conao/ghq/github.com/conao3/sanchoris",
        default_branch: "main",
        worktree_policy: "1 task = 1 branch = 1 worktree",
        dev_command: "pnpm dev",
        check_command: "cargo check --workspace",
        worker_policy: "Codex worker in task-scoped worktree",
        allowed_workflow: "workflow_mvp_delivery_v1",
    }
}

fn sample_tasks() -> TaskSnapshot {
    TaskSnapshot {
        conversation: Conversation {
            id: "conversation_builtin_chat",
            title: "Built-in chat intake",
            channel: "web",
        },
        messages: vec![ChatMessage {
            id: "message_001",
            conversation_id: "conversation_builtin_chat",
            author: "human",
            body: "Create a native Sanchoris task and run the MVP delivery workflow.",
            created_at: "2026-05-06T00:00:00Z",
            task_id: Some("task_native_001"),
        }],
        tasks: vec![NativeTask {
            id: "task_native_001",
            conversation_id: "conversation_builtin_chat",
            source_message_id: "message_001",
            title: "Implement a small Sanchoris change",
            description: "Native task created from the built-in chat and assigned to the MVP workflow.",
            status: "in_review",
            priority: "normal",
            project_id: "project_sanchoris",
            assigned_workflow_id: "workflow_mvp_delivery_v1",
            assigned_worker: "codex",
            workspace_id: "workspace_task_native_001",
            latest_run_id: "run_task_native_001",
            review_state: "gate_pending",
        }],
    }
}

fn sample_workflow() -> WorkflowSpec {
    WorkflowSpec {
        id: "workflow_mvp_delivery_v1",
        name: "MVP delivery workflow",
        version: "1",
        format: "yaml",
        yaml: "id: workflow_mvp_delivery_v1\nversion: 1\nblocks:\n  - ChatInput\n  - CreateTask\n  - CreateWorkspace\n  - RunWorker\n  - RunVerification\n  - Gate\n  - CreatePR\n  - Merge\n",
        blocks: vec![
            WorkflowBlock {
                id: "block_chat_input",
                kind: "ChatInput",
                label: "Built-in chat input",
                state: "completed",
                x: 0,
                y: 0,
            },
            WorkflowBlock {
                id: "block_create_task",
                kind: "CreateTask",
                label: "Create native task",
                state: "completed",
                x: 220,
                y: 0,
            },
            WorkflowBlock {
                id: "block_create_workspace",
                kind: "CreateWorkspace",
                label: "Create isolated workspace",
                state: "completed",
                x: 440,
                y: 0,
            },
            WorkflowBlock {
                id: "block_run_worker",
                kind: "RunWorker",
                label: "Run Codex worker",
                state: "completed",
                x: 660,
                y: 0,
            },
            WorkflowBlock {
                id: "block_run_verification",
                kind: "RunVerification",
                label: "Run verification",
                state: "completed",
                x: 880,
                y: 0,
            },
            WorkflowBlock {
                id: "block_gate",
                kind: "Gate",
                label: "Human review gate",
                state: "pending",
                x: 1100,
                y: 0,
            },
            WorkflowBlock {
                id: "block_create_pr",
                kind: "CreatePR",
                label: "Create pull request",
                state: "blocked",
                x: 1320,
                y: 0,
            },
            WorkflowBlock {
                id: "block_merge",
                kind: "Merge",
                label: "Merge pull request",
                state: "blocked",
                x: 1540,
                y: 0,
            },
        ],
        edges: vec![
            WorkflowEdge {
                from: "block_chat_input",
                to: "block_create_task",
            },
            WorkflowEdge {
                from: "block_create_task",
                to: "block_create_workspace",
            },
            WorkflowEdge {
                from: "block_create_workspace",
                to: "block_run_worker",
            },
            WorkflowEdge {
                from: "block_run_worker",
                to: "block_run_verification",
            },
            WorkflowEdge {
                from: "block_run_verification",
                to: "block_gate",
            },
            WorkflowEdge {
                from: "block_gate",
                to: "block_create_pr",
            },
            WorkflowEdge {
                from: "block_create_pr",
                to: "block_merge",
            },
        ],
    }
}

fn sample_runs() -> RunSnapshot {
    RunSnapshot {
        runs: vec![WorkerRun {
            id: "run_task_native_001",
            task_id: "task_native_001",
            worker_kind: "codex",
            status: "gate_pending",
            started_at: "2026-05-06T00:01:00Z",
            finished_at: Some("2026-05-06T00:08:00Z"),
            exit_code: Some(0),
            commit_hash: Some("0123456789abcdef0123456789abcdef01234567"),
            log_path: "s3://sanchoris-mvp/runs/run_task_native_001/transcript.log",
            error_summary: None,
            workspace: Workspace {
                id: "workspace_task_native_001",
                branch: "task/task_native_001",
                worktree_path: "/workspaces/sanchoris/task/task_native_001",
                base_commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                current_commit: Some("0123456789abcdef0123456789abcdef01234567"),
                cleanup_state: "retained_for_review",
                changed_files: &["apps/frontend/src/App.tsx"],
                verification_commands: &["cargo check --workspace"],
            },
            verification: VerificationResult {
                status: "passed",
                command: "cargo check --workspace",
                exit_code: Some(0),
                summary: "Workspace verification completed successfully.",
                artifact_uri: "s3://sanchoris-mvp/runs/run_task_native_001/verification.txt",
            },
            pull_request: PullRequest {
                url: None,
                source_branch: "task/task_native_001",
                base_branch: "main",
                status: "waiting_for_gate",
            },
            merge: MergeResult {
                method: "squash",
                status: "blocked",
                merge_commit: None,
                merged_at: None,
                merged_by: None,
                failure_reason: None,
            },
            gate: GateState {
                id: "gate_create_pr_task_native_001",
                state: "pending",
                review_target: "CreatePR",
                approver: None,
                decided_at: None,
            },
        }],
    }
}

fn json_array(values: impl Iterator<Item = String>) -> String {
    format!("[{}]", values.collect::<Vec<_>>().join(","))
}

fn json_optional_i32(value: Option<i32>) -> String {
    value.map_or_else(|| "null".to_string(), |number| number.to_string())
}

fn json_optional_string(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_string(), json_string)
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');

    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0C}' => escaped.push_str("\\f"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }

    escaped.push('"');
    escaped
}
