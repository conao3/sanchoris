use async_graphql::{
    Context, EmptySubscription, Enum, InputObject, Object, Request, Schema, SimpleObject,
};
use futures_executor::block_on;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

const JSON_CONTENT_TYPE: &str = "application/json";
const GRAPHQL_PATH: &str = "/api/graphql";

type SanchorisSchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;

fn main() -> std::io::Result<()> {
    let address = backend_address();
    let listener = TcpListener::bind(&address)?;
    let schema = build_schema();

    println!("sanchoris-backend listening on http://{address}");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => handle_connection(stream, &schema)?,
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

fn build_schema() -> SanchorisSchema {
    Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .data(Store::sample())
        .finish()
}

fn handle_connection(mut stream: TcpStream, schema: &SanchorisSchema) -> std::io::Result<()> {
    let request = read_http_request(&mut stream)?;
    let (method, path) = request_line(&request);

    let (status, content_type, body) = match (method, path) {
        ("GET", "/health") | ("GET", "/api/v1/health") => (
            "200 OK",
            JSON_CONTENT_TYPE,
            r#"{"status":"ok","service":"sanchoris-backend"}"#.to_string(),
        ),
        ("GET", GRAPHQL_PATH) => ("200 OK", JSON_CONTENT_TYPE, schema_sdl_response(schema)),
        ("POST", GRAPHQL_PATH) => execute_graphql(schema, &request),
        _ => (
            "404 Not Found",
            JSON_CONTENT_TYPE,
            r#"{"error":"not_found","service":"sanchoris-backend"}"#.to_string(),
        ),
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())
}

fn read_http_request(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buffer = vec![0; 65_536];
    let bytes_read = stream.read(&mut buffer)?;
    Ok(String::from_utf8_lossy(&buffer[..bytes_read]).to_string())
}

fn request_line(request: &str) -> (&str, &str) {
    let mut parts = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default();
    (method, path)
}

fn request_body(request: &str) -> &str {
    request.split("\r\n\r\n").nth(1).unwrap_or_default()
}

fn schema_sdl_response(schema: &SanchorisSchema) -> String {
    serde_json::json!({ "schema": schema.sdl() }).to_string()
}

fn execute_graphql(
    schema: &SanchorisSchema,
    http_request: &str,
) -> (&'static str, &'static str, String) {
    let body = request_body(http_request);
    let graphql_request = match parse_graphql_request(body) {
        Ok(request) => request,
        Err(error) => {
            return (
                "400 Bad Request",
                JSON_CONTENT_TYPE,
                serde_json::json!({ "errors": [{ "message": error }] }).to_string(),
            );
        }
    };

    let response = block_on(schema.execute(graphql_request));
    let body = serde_json::to_string(&response).unwrap_or_else(|error| {
        serde_json::json!({ "errors": [{ "message": format!("failed to serialize GraphQL response: {error}") }] }).to_string()
    });

    ("200 OK", JSON_CONTENT_TYPE, body)
}

fn parse_graphql_request(body: &str) -> Result<Request, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("invalid JSON body: {error}"))?;
    let query = value
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing GraphQL query".to_string())?;
    let mut request = Request::new(query);

    if let Some(operation_name) = value.get("operationName").and_then(Value::as_str) {
        request = request.operation_name(operation_name);
    }

    if let Some(variables) = value.get("variables").and_then(Value::as_object) {
        let variables = async_graphql::Variables::from_json(Value::Object(variables.clone()));
        request = request.variables(variables);
    }

    Ok(request)
}

#[derive(Clone)]
struct Store {
    viewer: Viewer,
    project_profiles: Vec<ProjectProfile>,
    conversations: Vec<Conversation>,
    tasks: Vec<NativeTask>,
    workflow_specs: Vec<WorkflowSpec>,
    runs: Vec<WorkerRun>,
}

impl Store {
    fn sample() -> Self {
        let workflow_yaml = r#"version: 1
name: Sanchoris MVP delivery
blocks:
  - ChatInput
  - CreateTask
  - CreateWorkspace
  - RunWorker
  - RunVerification
  - Gate
  - CreatePR
  - Merge
"#;

        Self {
            viewer: Viewer {
                id: "viewer_local".to_string(),
                display_name: "Local operator".to_string(),
                email: "local@sanchoris.dev".to_string(),
            },
            project_profiles: vec![ProjectProfile {
                id: "project_sanchoris".to_string(),
                name: "Sanchoris".to_string(),
                repository_path: "/home/conao/ghq/github.com/conao3/sanchoris".to_string(),
                default_branch: "master".to_string(),
                worktree_policy: "1 task = 1 branch = 1 worktree".to_string(),
                dev_command: "pnpm dev".to_string(),
                check_command: "pnpm check && cargo check --workspace".to_string(),
                worker_policy: "Codex CLI in an isolated Worktrunk worktree".to_string(),
                allowed_workflow_id: "workflow_mvp_delivery".to_string(),
            }],
            conversations: vec![Conversation {
                id: "conversation_builtin_chat".to_string(),
                title: "Built-in MVP chat".to_string(),
                channel: "built-in-chat".to_string(),
                messages: vec![
                    ChatMessage {
                        id: "message_001".to_string(),
                        author: "user".to_string(),
                        body: "Create a native task and run it with Codex in an isolated worktree."
                            .to_string(),
                        created_at: "2026-05-06T00:00:00Z".to_string(),
                        task_id: Some("task_native_001".to_string()),
                    },
                    ChatMessage {
                        id: "message_002".to_string(),
                        author: "sanchoris".to_string(),
                        body: "Task created, workflow assigned, and worker run queued.".to_string(),
                        created_at: "2026-05-06T00:00:05Z".to_string(),
                        task_id: Some("task_native_001".to_string()),
                    },
                ],
            }],
            tasks: vec![NativeTask {
                id: "task_native_001".to_string(),
                title: "Build MVP delivery shell".to_string(),
                description: "Track the built-in chat to native task to worktree and review flow."
                    .to_string(),
                status: TaskStatus::Review,
                priority: Priority::High,
                project_id: "project_sanchoris".to_string(),
                conversation_id: "conversation_builtin_chat".to_string(),
                assigned_workflow_id: "workflow_mvp_delivery".to_string(),
                assigned_worker: "codex".to_string(),
                workspace_id: "workspace_task_native_001".to_string(),
                latest_run_id: "run_task_native_001".to_string(),
                review_state: "gate_pending".to_string(),
            }],
            workflow_specs: vec![WorkflowSpec {
                id: "workflow_mvp_delivery".to_string(),
                name: "MVP delivery workflow".to_string(),
                version: "1".to_string(),
                yaml: workflow_yaml.to_string(),
                blocks: vec![
                    WorkflowBlock::new(
                        "block_chat_input",
                        "ChatInput",
                        "Capture message",
                        StepState::Passed,
                        0,
                        0,
                    ),
                    WorkflowBlock::new(
                        "block_create_task",
                        "CreateTask",
                        "Create native task",
                        StepState::Passed,
                        220,
                        0,
                    ),
                    WorkflowBlock::new(
                        "block_create_workspace",
                        "CreateWorkspace",
                        "Create branch and worktree",
                        StepState::Passed,
                        440,
                        0,
                    ),
                    WorkflowBlock::new(
                        "block_run_worker",
                        "RunWorker",
                        "Run Codex worker",
                        StepState::Passed,
                        660,
                        0,
                    ),
                    WorkflowBlock::new(
                        "block_run_verification",
                        "RunVerification",
                        "Run verification",
                        StepState::Passed,
                        880,
                        0,
                    ),
                    WorkflowBlock::new(
                        "block_gate",
                        "Gate",
                        "Human review gate",
                        StepState::Pending,
                        1100,
                        0,
                    ),
                    WorkflowBlock::new(
                        "block_create_pr",
                        "CreatePR",
                        "Create pull request",
                        StepState::Blocked,
                        1320,
                        0,
                    ),
                    WorkflowBlock::new(
                        "block_merge",
                        "Merge",
                        "Merge pull request",
                        StepState::Blocked,
                        1540,
                        0,
                    ),
                ],
                edges: vec![
                    WorkflowEdge::new("block_chat_input", "block_create_task"),
                    WorkflowEdge::new("block_create_task", "block_create_workspace"),
                    WorkflowEdge::new("block_create_workspace", "block_run_worker"),
                    WorkflowEdge::new("block_run_worker", "block_run_verification"),
                    WorkflowEdge::new("block_run_verification", "block_gate"),
                    WorkflowEdge::new("block_gate", "block_create_pr"),
                    WorkflowEdge::new("block_create_pr", "block_merge"),
                ],
            }],
            runs: vec![WorkerRun {
                id: "run_task_native_001".to_string(),
                task_id: "task_native_001".to_string(),
                worker_kind: "codex".to_string(),
                status: RunStatus::GatePending,
                started_at: "2026-05-06T00:01:00Z".to_string(),
                finished_at: Some("2026-05-06T00:08:00Z".to_string()),
                exit_code: Some(0),
                commit_hash: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
                log_uri: "s3://sanchoris-mvp/runs/run_task_native_001/transcript.log".to_string(),
                error_summary: None,
                workspace: Workspace {
                    id: "workspace_task_native_001".to_string(),
                    branch: "task/task_native_001".to_string(),
                    worktree_path: "/workspaces/sanchoris.task-native-001".to_string(),
                    base_commit: "d098afa".to_string(),
                    current_commit: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
                    cleanup_state: "retained_for_review".to_string(),
                },
                verification: VerificationResult {
                    command: "pnpm check && cargo check --workspace".to_string(),
                    exit_code: Some(0),
                    status: StepState::Passed,
                    summary: "Workspace verification completed successfully.".to_string(),
                    artifact_uri: "s3://sanchoris-mvp/runs/run_task_native_001/verification.txt"
                        .to_string(),
                },
                pull_request: PullRequest {
                    url: None,
                    source_branch: "task/task_native_001".to_string(),
                    base_branch: "master".to_string(),
                    status: "waiting_for_gate".to_string(),
                },
                merge: MergeResult {
                    method: "squash".to_string(),
                    status: "blocked".to_string(),
                    merge_commit: None,
                    merged_at: None,
                    merged_by: None,
                    failure_reason: None,
                },
                gate: GateState {
                    id: "gate_create_pr_task_native_001".to_string(),
                    state: "pending".to_string(),
                    review_target: "CreatePR".to_string(),
                    approver: None,
                    decided_at: None,
                },
            }],
        }
    }
}

struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn viewer(&self, ctx: &Context<'_>) -> Viewer {
        ctx.data_unchecked::<Store>().viewer.clone()
    }

    async fn project_profiles(&self, ctx: &Context<'_>) -> Vec<ProjectProfile> {
        ctx.data_unchecked::<Store>().project_profiles.clone()
    }

    async fn project_profile(&self, ctx: &Context<'_>, id: String) -> Option<ProjectProfile> {
        ctx.data_unchecked::<Store>()
            .project_profiles
            .iter()
            .find(|project| project.id == id)
            .cloned()
    }

    async fn conversations(&self, ctx: &Context<'_>) -> Vec<Conversation> {
        ctx.data_unchecked::<Store>().conversations.clone()
    }

    async fn tasks(&self, ctx: &Context<'_>) -> Vec<NativeTask> {
        ctx.data_unchecked::<Store>().tasks.clone()
    }

    async fn workflow_specs(&self, ctx: &Context<'_>) -> Vec<WorkflowSpec> {
        ctx.data_unchecked::<Store>().workflow_specs.clone()
    }

    async fn runs(&self, ctx: &Context<'_>) -> Vec<WorkerRun> {
        ctx.data_unchecked::<Store>().runs.clone()
    }
}

struct MutationRoot;

#[Object]
impl MutationRoot {
    async fn create_task(&self, _ctx: &Context<'_>, input: CreateTaskInput) -> NativeTask {
        NativeTask {
            id: "task_preview_created".to_string(),
            title: input.title,
            description: input.description,
            status: TaskStatus::Queued,
            priority: input.priority.unwrap_or(Priority::Medium),
            project_id: input.project_id,
            conversation_id: input.conversation_id,
            assigned_workflow_id: input.workflow_id,
            assigned_worker: input.worker.unwrap_or_else(|| "codex".to_string()),
            workspace_id: "workspace_pending".to_string(),
            latest_run_id: "run_pending".to_string(),
            review_state: "not_started".to_string(),
        }
    }

    async fn update_task_status(
        &self,
        ctx: &Context<'_>,
        id: String,
        status: TaskStatus,
    ) -> Option<NativeTask> {
        ctx.data_unchecked::<Store>()
            .tasks
            .iter()
            .find(|task| task.id == id)
            .cloned()
            .map(|mut task| {
                task.status = status;
                task
            })
    }

    async fn validate_workflow_canvas(
        &self,
        _ctx: &Context<'_>,
        workflow_id: String,
    ) -> WorkflowValidation {
        WorkflowValidation {
            workflow_id,
            valid: true,
            errors: Vec::new(),
        }
    }

    async fn start_worker_run(&self, ctx: &Context<'_>, task_id: String) -> Option<WorkerRun> {
        ctx.data_unchecked::<Store>()
            .runs
            .iter()
            .find(|run| run.task_id == task_id)
            .cloned()
    }

    async fn create_pull_request(&self, ctx: &Context<'_>, run_id: String) -> Option<PullRequest> {
        ctx.data_unchecked::<Store>()
            .runs
            .iter()
            .find(|run| run.id == run_id)
            .map(|run| run.pull_request.clone())
    }

    async fn merge_pull_request(&self, ctx: &Context<'_>, run_id: String) -> Option<MergeResult> {
        ctx.data_unchecked::<Store>()
            .runs
            .iter()
            .find(|run| run.id == run_id)
            .map(|run| run.merge.clone())
    }
}

#[derive(InputObject)]
struct CreateTaskInput {
    conversation_id: String,
    project_id: String,
    workflow_id: String,
    title: String,
    description: String,
    priority: Option<Priority>,
    worker: Option<String>,
}

#[derive(Clone, SimpleObject)]
struct Viewer {
    id: String,
    display_name: String,
    email: String,
}

#[derive(Clone, SimpleObject)]
struct ProjectProfile {
    id: String,
    name: String,
    repository_path: String,
    default_branch: String,
    worktree_policy: String,
    dev_command: String,
    check_command: String,
    worker_policy: String,
    allowed_workflow_id: String,
}

#[derive(Clone, SimpleObject)]
struct Conversation {
    id: String,
    title: String,
    channel: String,
    messages: Vec<ChatMessage>,
}

#[derive(Clone, SimpleObject)]
struct ChatMessage {
    id: String,
    author: String,
    body: String,
    created_at: String,
    task_id: Option<String>,
}

#[derive(Clone, SimpleObject)]
struct NativeTask {
    id: String,
    title: String,
    description: String,
    status: TaskStatus,
    priority: Priority,
    project_id: String,
    conversation_id: String,
    assigned_workflow_id: String,
    assigned_worker: String,
    workspace_id: String,
    latest_run_id: String,
    review_state: String,
}

#[derive(Clone, SimpleObject)]
struct WorkflowSpec {
    id: String,
    name: String,
    version: String,
    yaml: String,
    blocks: Vec<WorkflowBlock>,
    edges: Vec<WorkflowEdge>,
}

#[derive(Clone, SimpleObject)]
struct WorkflowBlock {
    id: String,
    kind: String,
    label: String,
    state: StepState,
    x: i32,
    y: i32,
}

impl WorkflowBlock {
    fn new(id: &str, kind: &str, label: &str, state: StepState, x: i32, y: i32) -> Self {
        Self {
            id: id.to_string(),
            kind: kind.to_string(),
            label: label.to_string(),
            state,
            x,
            y,
        }
    }
}

#[derive(Clone, SimpleObject)]
struct WorkflowEdge {
    from: String,
    to: String,
}

impl WorkflowEdge {
    fn new(from: &str, to: &str) -> Self {
        Self {
            from: from.to_string(),
            to: to.to_string(),
        }
    }
}

#[derive(Clone, SimpleObject)]
struct WorkerRun {
    id: String,
    task_id: String,
    worker_kind: String,
    status: RunStatus,
    started_at: String,
    finished_at: Option<String>,
    exit_code: Option<i32>,
    commit_hash: Option<String>,
    log_uri: String,
    error_summary: Option<String>,
    workspace: Workspace,
    verification: VerificationResult,
    pull_request: PullRequest,
    merge: MergeResult,
    gate: GateState,
}

#[derive(Clone, SimpleObject)]
struct Workspace {
    id: String,
    branch: String,
    worktree_path: String,
    base_commit: String,
    current_commit: Option<String>,
    cleanup_state: String,
}

#[derive(Clone, SimpleObject)]
struct VerificationResult {
    command: String,
    exit_code: Option<i32>,
    status: StepState,
    summary: String,
    artifact_uri: String,
}

#[derive(Clone, SimpleObject)]
struct PullRequest {
    url: Option<String>,
    source_branch: String,
    base_branch: String,
    status: String,
}

#[derive(Clone, SimpleObject)]
struct MergeResult {
    method: String,
    status: String,
    merge_commit: Option<String>,
    merged_at: Option<String>,
    merged_by: Option<String>,
    failure_reason: Option<String>,
}

#[derive(Clone, SimpleObject)]
struct GateState {
    id: String,
    state: String,
    review_target: String,
    approver: Option<String>,
    decided_at: Option<String>,
}

#[derive(Clone, SimpleObject)]
struct WorkflowValidation {
    workflow_id: String,
    valid: bool,
    errors: Vec<String>,
}

#[derive(Clone, Copy, Enum, Eq, PartialEq)]
enum Priority {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Enum, Eq, PartialEq)]
enum TaskStatus {
    Queued,
    Running,
    Review,
    Done,
    Failed,
}

#[derive(Clone, Copy, Enum, Eq, PartialEq)]
enum RunStatus {
    Queued,
    Running,
    GatePending,
    Passed,
    Failed,
}

#[derive(Clone, Copy, Enum, Eq, PartialEq)]
enum StepState {
    Pending,
    Running,
    Passed,
    Failed,
    Blocked,
}
