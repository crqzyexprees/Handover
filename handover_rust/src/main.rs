// Handlers return `Result<_, Response>` (axum's idiom); the large Err is fine.
#![allow(clippy::result_large_err)]

mod auth;
mod docker;
mod governor;
mod handoff;
mod handoff_flow;
mod local_config;
mod model;
mod process_stats;
mod projects;
mod pty;
mod sandbox;
mod state;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use clap::Parser;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::warn;
use tracing_subscriber::EnvFilter;

use crate::docker::DockerRuntime;
use crate::handoff_flow::{HandoffFailure, HandoffPlan, PromptSink};
use crate::model::{
    HandoffMethod, Instance, ProjectConfig, ResourceStats, SandboxMode, DEFAULT_MEM_LIMIT,
};
use crate::projects::{ProjectError, ProjectView, Projects};
use crate::pty::Terminals;
use crate::sandbox::{DockerSandbox, NativeSandbox, SandboxError, Sandboxes};
use crate::state::{monotonic_secs, RAM_EMERGENCY_THRESHOLD, RAM_SUSPEND_THRESHOLD};

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

#[derive(Parser)]
#[command(name = "handover-backend")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8765)]
    port: u16,
    /// Exit after this many seconds with no running shells and no requests.
    /// The desktop app sets it so a backend it leaves running (to keep
    /// shells alive across app restarts) doesn't outlive its usefulness.
    #[arg(long)]
    idle_exit_secs: Option<u64>,
}

/// Unix seconds of the last authenticated request (see `--idle-exit-secs`).
#[derive(Default)]
struct Activity(AtomicU64);

impl Activity {
    fn touch(&self) {
        self.0.store(monotonic_secs() as u64, Ordering::Relaxed);
    }
    fn idle_secs(&self) -> u64 {
        (monotonic_secs() as u64).saturating_sub(self.0.load(Ordering::Relaxed))
    }
}

#[derive(Clone)]
struct ServerCtx {
    projects: Arc<Projects>,
    terminals: Arc<Terminals>,
    activity: Arc<Activity>,
    /// Notified to stop the server (see `/api/shutdown`).
    shutdown: Arc<tokio::sync::Notify>,
}

fn api_err(status: StatusCode, detail: impl Into<String>) -> Response {
    (status, Json(json!({ "detail": detail.into() }))).into_response()
}

fn sandbox_err(e: SandboxError) -> Response {
    let status = match e {
        SandboxError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        SandboxError::Failed(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    api_err(status, e.to_string())
}

fn project_err(e: ProjectError) -> Response {
    match e {
        ProjectError::ProjectNotFound | ProjectError::InstanceNotFound => {
            api_err(StatusCode::NOT_FOUND, e.to_string())
        }
        ProjectError::Sandbox(e) => sandbox_err(e),
        ProjectError::Storage(_) => api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

fn validate_project_path(path: &str) -> Result<String, Response> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(api_err(StatusCode::BAD_REQUEST, "Project path is required"));
    }

    let canonical = std::fs::canonicalize(trimmed).map_err(|e| {
        api_err(
            StatusCode::BAD_REQUEST,
            format!("Project path does not exist or is not readable: {e}"),
        )
    })?;

    if !canonical.is_dir() {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "Project path must be a directory",
        ));
    }

    canonical
        .to_str()
        .map(String::from)
        .ok_or_else(|| api_err(StatusCode::BAD_REQUEST, "Project path is not valid UTF-8"))
}

/// Parses one of an enum's wire names, e.g. `"native"`, into the enum.
fn parse_choice<T: DeserializeOwned>(field: &str, value: &str) -> Result<T, Response> {
    serde_json::from_value(Value::String(value.into()))
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, format!("{field}: {e}")))
}

fn parse_choice_or_default<T: DeserializeOwned + Default>(
    field: &str,
    value: Option<&str>,
) -> Result<T, Response> {
    value.map_or_else(|| Ok(T::default()), |v| parse_choice(field, v))
}

#[derive(Deserialize)]
struct ProjectCreate {
    path: String,
    sandbox_mode: Option<String>,
}

#[derive(Deserialize)]
struct ProjectConfigBody {
    project_name: Option<String>,
    sandbox_mode: Option<String>,
    mem_limit: Option<String>,
    handoff_method: Option<String>,
    handoff_template: Option<String>,
    custom_env_vars: Option<BTreeMap<String, String>>,
}

/// Everything else (memory limit, env vars, default mode) comes from the
/// Project's effective config, so `.handover/config.yml` edits apply.
#[derive(Deserialize)]
struct InstanceStart {
    project_id: String,
    /// An explicit choice (the Native / Docker button) over the config's mode.
    sandbox_mode: Option<String>,
}

#[derive(Deserialize)]
struct HandoffRequest {
    from_instance_id: String,
    to_instance_id: String,
    project_id: String,
    method: String,
    task_description: String,
}

async fn root() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "handover-backend",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Authenticated liveness check; the desktop app polls it as a heartbeat.
async fn ping(State(ctx): State<ServerCtx>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "shells": ctx.terminals.running().await,
    }))
}

#[derive(Deserialize)]
struct ShutdownBody {
    #[serde(default = "default_true")]
    only_if_idle: bool,
}

fn default_true() -> bool {
    true
}

/// Stops the backend; with `only_if_idle`, only when no shell is running.
/// The desktop app calls this on quit, so running shells survive the quit.
async fn shutdown(State(ctx): State<ServerCtx>, Json(body): Json<ShutdownBody>) -> Json<Value> {
    let shells = ctx.terminals.running().await;
    if body.only_if_idle && shells > 0 {
        return Json(json!({ "stopping": false, "shells": shells }));
    }
    let signal = ctx.shutdown.clone();
    tokio::spawn(async move {
        // Let this response reach the caller first.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        signal.notify_one();
    });
    Json(json!({ "stopping": true, "shells": shells }))
}

async fn track_activity(
    State(activity): State<Arc<Activity>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if req.uri().path() != "/" {
        activity.touch();
    }
    next.run(req).await
}

async fn docker_health(State(ctx): State<ServerCtx>) -> Json<Value> {
    match ctx.projects.sandbox_health(SandboxMode::Docker).await {
        Ok(()) => Json(json!({ "status": "ok" })),
        Err(_) => Json(json!({
            "status": "error",
            "message": "Docker is not running or not installed."
        })),
    }
}

async fn get_config(State(ctx): State<ServerCtx>) -> Json<Value> {
    Json(ctx.projects.app_config().await)
}

async fn update_config(
    State(ctx): State<ServerCtx>,
    Json(patch): Json<BTreeMap<String, Value>>,
) -> Json<Value> {
    ctx.projects.update_app_config(patch).await;
    Json(json!({ "status": "ok" }))
}

/// Projects with their effective config and open instances, so a reopened
/// window can re-attach to terminals whose shells kept running.
async fn list_projects(State(ctx): State<ServerCtx>) -> Json<Vec<ProjectView>> {
    Json(ctx.projects.list().await)
}

async fn create_project(
    State(ctx): State<ServerCtx>,
    Json(body): Json<ProjectCreate>,
) -> Result<Json<ProjectView>, Response> {
    let sandbox_mode: SandboxMode =
        parse_choice_or_default("sandbox_mode", body.sandbox_mode.as_deref())?;
    let path = validate_project_path(&body.path)?;
    let view = ctx
        .projects
        .create(path, sandbox_mode)
        .await
        .map_err(project_err)?;
    Ok(Json(view))
}

async fn get_project_config(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
) -> Result<Json<ProjectConfig>, Response> {
    let config = ctx
        .projects
        .effective_config(&project_id)
        .await
        .map_err(project_err)?;
    Ok(Json(config))
}

async fn save_project_config(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
    Json(body): Json<ProjectConfigBody>,
) -> Result<Json<ProjectConfig>, Response> {
    let config = ProjectConfig {
        project_name: body.project_name.unwrap_or_default().trim().to_string(),
        sandbox_mode: parse_choice_or_default("sandbox_mode", body.sandbox_mode.as_deref())?,
        mem_limit: body
            .mem_limit
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_MEM_LIMIT.into()),
        handoff_method: parse_choice_or_default("handoff_method", body.handoff_method.as_deref())?,
        handoff_template: parse_choice_or_default(
            "handoff_template",
            body.handoff_template.as_deref(),
        )?,
        custom_env_vars: body.custom_env_vars.unwrap_or_default(),
    };
    let saved = ctx
        .projects
        .configure(&project_id, config)
        .await
        .map_err(project_err)?;
    Ok(Json(saved))
}

async fn activate_project(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
) -> Result<Json<Value>, Response> {
    ctx.projects
        .activate(&project_id)
        .await
        .map_err(project_err)?;
    Ok(Json(json!({ "status": "ok" })))
}

async fn resume_project(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
) -> Result<Json<Value>, Response> {
    ctx.projects
        .resume(&project_id)
        .await
        .map_err(project_err)?;
    Ok(Json(json!({ "status": "ok", "state": "active" })))
}

async fn unload_project(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
) -> Json<Value> {
    let message = if ctx.projects.unload(&project_id).await {
        "Project unloaded"
    } else {
        "Already unloaded"
    };
    Json(json!({ "status": "ok", "message": message }))
}

async fn start_instance(
    State(ctx): State<ServerCtx>,
    Json(body): Json<InstanceStart>,
) -> Result<Json<Instance>, Response> {
    let mode = body
        .sandbox_mode
        .as_deref()
        .map(|m| parse_choice::<SandboxMode>("sandbox_mode", m))
        .transpose()?;
    let instance = ctx
        .projects
        .open_instance(&body.project_id, mode)
        .await
        .map_err(project_err)?;
    Ok(Json(instance))
}

async fn delete_instance(
    State(ctx): State<ServerCtx>,
    Path(instance_id): Path<String>,
) -> Json<Value> {
    ctx.projects.close_instance(&instance_id).await;
    Json(json!({ "status": "ok" }))
}

async fn instance_stats(
    State(ctx): State<ServerCtx>,
    Path(instance_id): Path<String>,
) -> Result<Json<ResourceStats>, Response> {
    let stats = ctx
        .projects
        .instance_stats(&instance_id)
        .await
        .map_err(project_err)?;
    Ok(Json(stats))
}

async fn focus_instance(
    State(ctx): State<ServerCtx>,
    Path(instance_id): Path<String>,
) -> Result<Json<Value>, Response> {
    ctx.projects
        .instance(&instance_id)
        .await
        .map_err(project_err)?;
    Ok(Json(json!({ "status": "ok" })))
}

/// Delivers handoff prompts to instances' running shells.
struct ShellPrompts<'a>(&'a Terminals);

impl PromptSink for ShellPrompts<'_> {
    async fn send_prompt(&self, instance_id: &str, prompt: &str) -> bool {
        self.0.type_prompt(instance_id, prompt).await
    }
}

async fn execute_handoff(
    State(ctx): State<ServerCtx>,
    Json(body): Json<HandoffRequest>,
) -> Result<Response, Response> {
    let method: HandoffMethod = parse_choice("method", &body.method)?;
    let project = ctx
        .projects
        .project(&body.project_id)
        .await
        .map_err(project_err)?;
    for (field, id) in [
        ("from_instance", &body.from_instance_id),
        ("to_instance", &body.to_instance_id),
    ] {
        let instance = ctx
            .projects
            .instance(id)
            .await
            .map_err(|_| api_err(StatusCode::NOT_FOUND, format!("{field} not found")))?;
        if instance.project_id != project.id {
            return Err(api_err(
                StatusCode::BAD_REQUEST,
                format!("{field} does not belong to project_id"),
            ));
        }
    }
    ctx.projects
        .ensure_active(&project.id)
        .await
        .map_err(project_err)?;
    let template = match method {
        HandoffMethod::Summary => {
            ctx.projects
                .effective_config(&project.id)
                .await
                .map_err(project_err)?
                .handoff_template
        }
        HandoffMethod::Git => Default::default(),
    };

    let plan = HandoffPlan {
        project_path: &project.path,
        from_instance_id: &body.from_instance_id,
        to_instance_id: &body.to_instance_id,
        method,
        template,
        task: &body.task_description,
        summary_wait: handoff::HANDOFF_FILE_WAIT_TIMEOUT,
    };
    match handoff_flow::run(plan, &ShellPrompts(&ctx.terminals)).await {
        Ok(outcome) => Ok(Json(json!({
            "status": "ok",
            "kind": outcome.kind(),
            "message": outcome.message(),
        }))
        .into_response()),
        Err(f @ (HandoffFailure::NotGitRepo | HandoffFailure::SummaryTimedOut)) => Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "status": "error", "kind": f.kind(), "message": f.message() })),
        )
            .into_response()),
        Err(f @ (HandoffFailure::SourceNotRunning | HandoffFailure::TargetNotRunning)) => {
            Err(api_err(StatusCode::BAD_REQUEST, f.message()))
        }
        Err(f @ HandoffFailure::Internal(_)) => {
            Err(api_err(StatusCode::INTERNAL_SERVER_ERROR, f.message()))
        }
    }
}

async fn project_path_for(ctx: &ServerCtx, project_id: &str) -> Result<String, Response> {
    Ok(ctx
        .projects
        .project(project_id)
        .await
        .map_err(project_err)?
        .path)
}

async fn list_project_handoffs(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
) -> Result<Json<Value>, Response> {
    let path = project_path_for(&ctx, &project_id).await?;
    let files = handoff::list_handoff_files(&path)
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "files": files })))
}

async fn export_project_handoffs(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, Response> {
    let path = project_path_for(&ctx, &project_id).await?;
    let markdown = handoff::export_handoff_log(&path)
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((
        [(
            axum::http::header::CONTENT_TYPE,
            "text/markdown; charset=utf-8",
        )],
        markdown,
    ))
}

async fn get_project_handoff_file(
    State(ctx): State<ServerCtx>,
    Path((project_id, filename)): Path<(String, String)>,
) -> Result<Json<Value>, Response> {
    if !handoff::is_valid_handoff_filename(&filename) {
        return Err(api_err(StatusCode::BAD_REQUEST, "Invalid handoff filename"));
    }
    let path = project_path_for(&ctx, &project_id).await?;
    let content = handoff::read_handoff_file(&path, &filename)
        .await
        .map_err(|e| api_err(StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(json!({ "filename": filename, "content": content })))
}

#[derive(Deserialize)]
struct DiffQuery {
    from: String,
    to: String,
}

async fn diff_project_handoffs(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
    Query(q): Query<DiffQuery>,
) -> Result<Json<Value>, Response> {
    let path = project_path_for(&ctx, &project_id).await?;
    let diff = handoff::diff_handoff_files(&path, &q.from, &q.to)
        .await
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!({ "from": q.from, "to": q.to, "diff": diff })))
}

async fn project_git_diff(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
    Query(q): Query<DiffQuery>,
) -> Result<Json<Value>, Response> {
    let path = project_path_for(&ctx, &project_id).await?;
    let diff = handoff::git_diff_range(&path, &q.from, &q.to)
        .await
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!({ "from": q.from, "to": q.to, "diff": diff })))
}

#[derive(Deserialize)]
struct BroadcastBody {
    prompt: String,
}

async fn broadcast_prompt(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
    Json(body): Json<BroadcastBody>,
) -> Result<Json<Value>, Response> {
    let prompt = body.prompt.trim();
    if prompt.is_empty() {
        return Err(api_err(StatusCode::BAD_REQUEST, "prompt is required"));
    }
    // A paused container would swallow the keystrokes.
    ctx.projects
        .ensure_active(&project_id)
        .await
        .map_err(project_err)?;

    let instances = ctx.projects.instances_of(&project_id).await;
    let mut sent = 0u32;
    for instance in &instances {
        if ctx.terminals.type_prompt(&instance.id, prompt).await {
            sent += 1;
        }
    }
    Ok(Json(
        json!({ "status": "ok", "sent": sent, "total": instances.len() }),
    ))
}

fn host_memory_mb() -> (f64, f64) {
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};
    match System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()),
    ) {
        Ok(mut sys) => {
            sys.refresh_memory();
            (
                sys.total_memory() as f64 / (1024.0 * 1024.0),
                sys.used_memory() as f64 / (1024.0 * 1024.0),
            )
        }
        Err(e) => {
            warn!("failed to read host memory: {e}");
            (0.0, 0.0)
        }
    }
}

async fn project_resources(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
) -> Result<Json<Value>, Response> {
    let project = ctx
        .projects
        .project(&project_id)
        .await
        .map_err(project_err)?;
    let (total_mb, used_mb) = tokio::task::spawn_blocking(host_memory_mb)
        .await
        .unwrap_or((0.0, 0.0));
    let ram_percent = if total_mb > 0.0 {
        used_mb / total_mb * 100.0
    } else {
        0.0
    };

    let rows: Vec<Value> = ctx
        .projects
        .resources(&project_id)
        .await
        .map_err(project_err)?
        .into_iter()
        .enumerate()
        .map(|(index, r)| {
            json!({
                "instance_id": r.instance.id,
                "label": format!("Terminal {}", index + 1),
                "sandbox_mode": r.instance.sandbox_mode,
                // True while the shell is running, attached window or not.
                "connected": r.shell_running,
                "stats": r.stats,
            })
        })
        .collect();

    Ok(Json(json!({
        "system": {
            "ram_used_mb": used_mb,
            "ram_total_mb": total_mb,
            "ram_percent": ram_percent,
            "suspend_threshold_percent": RAM_SUSPEND_THRESHOLD,
            "emergency_threshold_percent": RAM_EMERGENCY_THRESHOLD,
        },
        "project": {
            "state": project.state,
            "instance_count": rows.len(),
        },
        "instances": rows,
    })))
}

async fn ws_pty(
    ws: WebSocketUpgrade,
    Path(instance_id): Path<String>,
    Query(params): Query<pty::AttachParams>,
    State(ctx): State<ServerCtx>,
) -> Result<impl IntoResponse, StatusCode> {
    if ctx.projects.instance(&instance_id).await.is_err() {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(ws.on_upgrade(move |socket| {
        let projects = ctx.projects.clone();
        let id = instance_id.clone();
        pty::handle_pty_socket(
            socket,
            instance_id,
            params,
            ctx.terminals,
            move || async move { projects.shell_command(&id).await },
        )
    }))
}

fn app(ctx: ServerCtx, token: auth::AuthToken) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/api/ping", get(ping))
        .route("/api/shutdown", post(shutdown))
        .route("/api/health/docker", get(docker_health))
        .route("/api/config", get(get_config).put(update_config))
        .route("/api/projects", get(list_projects).post(create_project))
        .route(
            "/api/projects/{project_id}/config",
            get(get_project_config).put(save_project_config),
        )
        .route(
            "/api/projects/{project_id}/activate",
            post(activate_project),
        )
        .route("/api/projects/{project_id}/resume", post(resume_project))
        .route("/api/projects/{project_id}/unload", post(unload_project))
        .route("/api/instances/start", post(start_instance))
        .route("/api/instances/{instance_id}", delete(delete_instance))
        .route("/api/instances/{instance_id}/stats", get(instance_stats))
        .route("/api/instances/{instance_id}/focus", post(focus_instance))
        .route("/api/handoff", post(execute_handoff))
        .route(
            "/api/projects/{project_id}/handoffs/diff",
            get(diff_project_handoffs),
        )
        .route(
            "/api/projects/{project_id}/handoffs/export",
            get(export_project_handoffs),
        )
        .route(
            "/api/projects/{project_id}/handoffs/{filename}",
            get(get_project_handoff_file),
        )
        .route(
            "/api/projects/{project_id}/handoffs",
            get(list_project_handoffs),
        )
        .route("/api/projects/{project_id}/git-diff", get(project_git_diff))
        .route(
            "/api/projects/{project_id}/resources",
            get(project_resources),
        )
        .route(
            "/api/projects/{project_id}/broadcast",
            post(broadcast_prompt),
        )
        .route("/ws/pty/{instance_id}", get(ws_pty))
        .with_state(ctx.clone())
        .layer(axum::middleware::from_fn_with_state(
            ctx.activity,
            track_activity,
        ))
        .layer(axum::middleware::from_fn_with_state(
            token,
            auth::require_token,
        ))
        .layer(auth::cors_layer())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();

    let docker_slot: Arc<RwLock<Option<Arc<DockerRuntime>>>> = Arc::new(RwLock::new(None));

    match DockerRuntime::new() {
        Ok(runtime) => {
            let arc = Arc::new(runtime);
            let removed = arc.cleanup_orphans().await;
            tracing::debug!("[startup] removed {removed} orphaned sandbox container(s)");
            let removed_nss = arc.cleanup_nss_temp_dirs().await;
            tracing::debug!("[startup] removed {removed_nss} stale nss temp dir(s)");
            *docker_slot.write().await = Some(arc);
        }
        Err(e) => {
            tracing::debug!("[startup] Docker unavailable; skipping container cleanup: {e}");
        }
    }
    let sandboxes = Sandboxes::new(
        Arc::new(NativeSandbox),
        Arc::new(DockerSandbox::new(docker_slot.clone())),
    );
    let terminals = Terminals::new();
    let projects = Projects::load(state::default_path(), sandboxes, terminals.clone()).await;
    governor::spawn_governor(projects.clone());

    let ctx = ServerCtx {
        projects,
        terminals,
        activity: Arc::new(Activity::default()),
        shutdown: Arc::new(tokio::sync::Notify::new()),
    };
    ctx.activity.touch();
    if let Some(idle) = cli.idle_exit_secs {
        spawn_idle_watchdog(&ctx, idle);
    }

    // Per-launch secret from the desktop shell (env, not argv, so other
    // local users can't read it from the process list).
    let token = match std::env::var("HANDOVER_TOKEN") {
        Ok(t) if !t.trim().is_empty() => auth::AuthToken(t.trim().into()),
        _ => {
            let t = auth::AuthToken::generate();
            // Standalone/dev runs: print it so a browser dev session can use
            // http://127.0.0.1:5173/?port=<port>&token=<token>
            println!("handover-backend token: {}", t.0);
            t
        }
    };

    let app = app(ctx.clone(), token);

    let addr: SocketAddr = format!("{}:{}", cli.host, cli.port).parse()?;
    tracing::debug!("handover-backend listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tokio::select! {
        served = std::future::IntoFuture::into_future(axum::serve(listener, app)) => served?,
        () = stop_requested(&ctx.shutdown) => {}
    }
    // Nothing is left running behind a backend that is gone.
    ctx.projects.shutdown_all().await;
    Ok(())
}

async fn stop_requested(shutdown: &tokio::sync::Notify) {
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = shutdown.notified() => {}
        _ = tokio::signal::ctrl_c() => {}
        () = term => {}
    }
}

fn spawn_idle_watchdog(ctx: &ServerCtx, idle_secs: u64) {
    let terminals = ctx.terminals.clone();
    let activity = ctx.activity.clone();
    let shutdown = ctx.shutdown.clone();
    tokio::spawn(async move {
        let check_every = std::time::Duration::from_secs(idle_secs.clamp(1, 30));
        loop {
            tokio::time::sleep(check_every).await;
            if terminals.running().await == 0 && activity.idle_secs() >= idle_secs {
                tracing::info!("no shells and no requests for {idle_secs}s; exiting");
                shutdown.notify_one();
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::fake::FakeSandbox;
    use axum::body::Body;
    use axum::http::Request;
    use std::path::PathBuf;
    use tower::ServiceExt;

    const TOKEN: &str = "test-token";

    struct Harness {
        _dir: tempfile::TempDir,
        project_dir: PathBuf,
        ctx: ServerCtx,
        docker: Arc<FakeSandbox>,
    }

    async fn harness(docker: FakeSandbox) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("proj");
        std::fs::create_dir(&project_dir).unwrap();
        let docker = Arc::new(docker);
        let sandboxes = Sandboxes::new(Arc::new(NativeSandbox), docker.clone());
        let terminals = Terminals::new();
        let projects =
            Projects::load(dir.path().join("state.json"), sandboxes, terminals.clone()).await;
        let ctx = ServerCtx {
            projects,
            terminals,
            activity: Arc::new(Activity::default()),
            shutdown: Arc::new(tokio::sync::Notify::new()),
        };
        Harness {
            _dir: dir,
            project_dir,
            ctx,
            docker,
        }
    }

    impl Harness {
        async fn call(&self, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
            let req = Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .header("x-handover-token", TOKEN)
                .body(Body::from(body.to_string()))
                .unwrap();
            let res = app(self.ctx.clone(), auth::AuthToken(TOKEN.into()))
                .oneshot(req)
                .await
                .unwrap();
            let status = res.status();
            let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap();
            (
                status,
                serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            )
        }

        async fn create_project(&self) -> String {
            let (status, p) = self
                .call(
                    "POST",
                    "/api/projects",
                    json!({ "path": self.project_dir, "sandbox_mode": "docker" }),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{p}");
            p["id"].as_str().unwrap().to_string()
        }
    }

    #[tokio::test]
    async fn docker_instance_is_provisioned_and_torn_down_through_the_sandbox() {
        let h = harness(FakeSandbox {
            container: Some("c1".into()),
            ..Default::default()
        })
        .await;
        let pid = h.create_project().await;
        let (status, inst) = h
            .call(
                "POST",
                "/api/instances/start",
                json!({ "project_id": pid, "sandbox_mode": "docker" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{inst}");
        assert_eq!(inst["container_id"], "c1");
        assert_eq!(inst["mem_limit"], "2g");
        let iid = inst["id"].as_str().unwrap();

        let (_, stats) = h
            .call("GET", &format!("/api/instances/{iid}/stats"), Value::Null)
            .await;
        assert_eq!(stats["cpu_percent"], 3.0);

        h.call("DELETE", &format!("/api/instances/{iid}"), Value::Null)
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            h.docker.calls(),
            [
                format!("provision {iid}"),
                format!("stats {iid}"),
                format!("teardown {iid}")
            ]
        );
    }

    #[tokio::test]
    async fn hand_edited_config_yml_drives_new_instances() {
        let h = harness(FakeSandbox {
            container: Some("c1".into()),
            ..Default::default()
        })
        .await;
        let pid = h.create_project().await;
        std::fs::write(
            h.project_dir.join(".handover/config.yml"),
            "sandbox_mode: docker\nmem_limit: 3g\ncustom_env_vars:\n  FOO: bar\n",
        )
        .unwrap();
        let (status, inst) = h
            .call("POST", "/api/instances/start", json!({ "project_id": pid }))
            .await;
        assert_eq!(status, StatusCode::OK, "{inst}");
        assert_eq!(inst["sandbox_mode"], "docker");
        assert_eq!(inst["mem_limit"], "3g");
        assert_eq!(inst["custom_env_vars"]["FOO"], "bar");
        let (_, list) = h.call("GET", "/api/projects", Value::Null).await;
        assert_eq!(list[0]["config"]["mem_limit"], "3g");
    }

    #[tokio::test]
    async fn explicit_mode_choice_overrides_the_config() {
        let h = harness(FakeSandbox::default()).await;
        let pid = h.create_project().await; // config says docker
        let (_, inst) = h
            .call(
                "POST",
                "/api/instances/start",
                json!({ "project_id": pid, "sandbox_mode": "native" }),
            )
            .await;
        assert_eq!(inst["sandbox_mode"], "native");
        assert!(h.docker.calls().is_empty());
    }

    #[tokio::test]
    async fn unavailable_sandbox_is_a_503() {
        let h = harness(FakeSandbox {
            fail_provision: true,
            ..Default::default()
        })
        .await;
        let pid = h.create_project().await;
        let (status, _) = h
            .call("POST", "/api/instances/start", json!({ "project_id": pid }))
            .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(!h.ctx.projects.has_instances().await);
    }

    #[tokio::test]
    async fn native_instances_never_touch_docker() {
        let h = harness(FakeSandbox::default()).await;
        let pid = h.create_project().await;
        let (status, inst) = h
            .call(
                "POST",
                "/api/instances/start",
                json!({ "project_id": pid, "sandbox_mode": "native" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(inst["container_id"], Value::Null);
        assert!(inst.get("mem_limit").is_none());
        h.call("POST", &format!("/api/projects/{pid}/unload"), Value::Null)
            .await;
        assert!(h.docker.calls().is_empty());
    }

    #[tokio::test]
    async fn unknown_choices_are_rejected_with_400() {
        let h = harness(FakeSandbox::default()).await;
        let pid = h.create_project().await;
        let (status, body) = h
            .call(
                "PUT",
                &format!("/api/projects/{pid}/config"),
                json!({ "sandbox_mode": "vm" }),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["detail"].as_str().unwrap().contains("sandbox_mode"));
    }

    #[tokio::test]
    async fn saved_config_round_trips_and_renames_the_project() {
        let h = harness(FakeSandbox::default()).await;
        let pid = h.create_project().await;
        let (status, _) = h
            .call(
                "PUT",
                &format!("/api/projects/{pid}/config"),
                json!({ "project_name": "Renamed", "sandbox_mode": "native", "handoff_template": "rust-cli" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        let (_, cfg) = h
            .call("GET", &format!("/api/projects/{pid}/config"), Value::Null)
            .await;
        assert_eq!(cfg["handoff_template"], "rust-cli");
        assert_eq!(cfg["sandbox_mode"], "native");
        let (_, list) = h.call("GET", "/api/projects", Value::Null).await;
        assert_eq!(list[0]["name"], "Renamed");
        assert_eq!(list[0]["config"]["project_name"], "Renamed");
    }

    #[tokio::test]
    async fn project_list_carries_open_instances_oldest_first() {
        let h = harness(FakeSandbox::default()).await;
        let pid = h.create_project().await;
        let mut opened = Vec::new();
        for _ in 0..3 {
            let (_, inst) = h
                .call(
                    "POST",
                    "/api/instances/start",
                    json!({ "project_id": pid, "sandbox_mode": "native" }),
                )
                .await;
            opened.push(inst["id"].as_str().unwrap().to_string());
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let (_, list) = h.call("GET", "/api/projects", Value::Null).await;
        let listed: Vec<&str> = list[0]["instances"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap())
            .collect();
        assert_eq!(listed, opened);
    }

    #[tokio::test]
    async fn idle_shutdown_signals_the_server_to_stop() {
        let h = harness(FakeSandbox::default()).await;
        let (_, body) = h.call("POST", "/api/shutdown", json!({})).await;
        assert_eq!(body["stopping"], true);
        tokio::time::timeout(std::time::Duration::from_secs(2), h.ctx.shutdown.notified())
            .await
            .expect("shutdown was not signalled");
    }

    #[tokio::test]
    async fn broadcast_resumes_a_suspended_project_first() {
        let h = harness(FakeSandbox {
            container: Some("c1".into()),
            ..Default::default()
        })
        .await;
        let pid = h.create_project().await;
        h.call("POST", "/api/instances/start", json!({ "project_id": pid }))
            .await;
        assert!(h.ctx.projects.suspend(&pid).await);
        let (status, _) = h
            .call(
                "POST",
                &format!("/api/projects/{pid}/broadcast"),
                json!({ "prompt": "hi" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert!(h.docker.calls().iter().any(|c| c.starts_with("resume ")));
    }

    #[tokio::test]
    async fn suspended_project_resumes_its_instances_on_activate() {
        let h = harness(FakeSandbox {
            container: Some("c1".into()),
            ..Default::default()
        })
        .await;
        let pid = h.create_project().await;
        h.call("POST", "/api/instances/start", json!({ "project_id": pid }))
            .await;
        assert!(h.ctx.projects.suspend(&pid).await);
        h.call(
            "POST",
            &format!("/api/projects/{pid}/activate"),
            Value::Null,
        )
        .await;
        assert!(h.docker.calls().iter().any(|c| c.starts_with("resume ")));
        assert_eq!(
            h.ctx.projects.project(&pid).await.unwrap().state,
            crate::model::ProjectState::Active
        );
    }

    #[test]
    fn validate_project_path_accepts_existing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let validated = validate_project_path(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(
            PathBuf::from(validated),
            std::fs::canonicalize(dir.path()).unwrap()
        );
    }

    #[test]
    fn validate_project_path_rejects_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert!(validate_project_path(missing.to_str().unwrap()).is_err());
    }
}
