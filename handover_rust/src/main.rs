mod docker;
mod governor;
mod handoff;
mod local_config;
mod pty;
mod state;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use clap::Parser;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tracing::warn;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::docker::DockerRuntime;
use crate::state::{
    basename_from_path, default_project_config, monotonic_secs, project_container_ids, AppState,
    DEFAULT_MEM_LIMIT, HANDOFF_METHODS, HANDOFF_TEMPLATES, RAM_EMERGENCY_THRESHOLD,
    RAM_SUSPEND_THRESHOLD, SANDBOX_MODES,
};

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
}

#[derive(Clone)]
struct ServerCtx {
    state: Arc<AppState>,
    docker: Arc<RwLock<Option<Arc<DockerRuntime>>>>,
}

fn api_err(status: StatusCode, detail: impl Into<String>) -> Response {
    (status, Json(json!({ "detail": detail.into() }))).into_response()
}

async fn persist_or_500(state: &AppState) -> Result<(), Response> {
    state.save_persisted().await.map_err(|e| {
        api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("state save failed: {e}"),
        )
    })
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

async fn get_docker(ctx: &ServerCtx) -> Result<Arc<DockerRuntime>, Response> {
    if let Some(runtime) = ctx.docker.read().await.clone() {
        return Ok(runtime);
    }

    let mut slot = ctx.docker.write().await;
    if let Some(runtime) = slot.clone() {
        return Ok(runtime);
    }

    let runtime = Arc::new(DockerRuntime::new().map_err(|e| {
        api_err(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("Docker is not available: {e}"),
        )
    })?);
    *slot = Some(runtime.clone());
    Ok(runtime)
}

fn sandbox_ok(mode: &str) -> bool {
    SANDBOX_MODES.contains(&mode)
}

#[derive(Deserialize)]
struct ProjectCreate {
    path: String,
    #[serde(default = "default_docker")]
    sandbox_mode: String,
}

fn default_docker() -> String {
    "docker".into()
}

#[derive(Deserialize)]
struct ProjectConfigBody {
    project_name: Option<String>,
    #[serde(default = "default_docker")]
    sandbox_mode: String,
    #[serde(default = "default_mem")]
    mem_limit: String,
    #[serde(default = "default_handoff_method")]
    handoff_method: String,
    #[serde(default = "default_handoff_template")]
    handoff_template: String,
    custom_env_vars: Option<HashMap<String, String>>,
}

fn default_handoff_template() -> String {
    "generic".into()
}

fn default_mem() -> String {
    DEFAULT_MEM_LIMIT.into()
}
fn default_handoff_method() -> String {
    "summary".into()
}

#[derive(Deserialize)]
struct InstanceStart {
    project_id: String,
    #[serde(default = "default_docker")]
    sandbox_mode: String,
    mem_limit: Option<String>,
    custom_env_vars: Option<HashMap<String, String>>,
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
    Json(json!({ "status": "ok", "service": "handover-backend" }))
}

async fn docker_health(State(ctx): State<ServerCtx>) -> Json<Value> {
    match ctx.docker.read().await.as_ref() {
        Some(d) if d.ping().await.is_ok() => Json(json!({ "status": "ok" })),
        _ => Json(json!({
            "status": "error",
            "message": "Docker is not running or not installed."
        })),
    }
}

async fn get_config(State(ctx): State<ServerCtx>) -> Json<Value> {
    Json(ctx.state.app_config.read().await.clone())
}

async fn update_config(State(ctx): State<ServerCtx>, Json(body): Json<Value>) -> Json<Value> {
    let mut cfg = ctx.state.app_config.write().await;
    if let (Some(base), Some(patch)) = (cfg.as_object_mut(), body.as_object()) {
        for (k, v) in patch {
            base.insert(k.clone(), v.clone());
        }
    }
    drop(cfg);
    if let Err(e) = ctx.state.save_persisted().await {
        warn!("failed to persist app config: {e}");
    }
    Json(json!({ "status": "ok" }))
}

async fn list_projects(State(ctx): State<ServerCtx>) -> Json<Vec<Value>> {
    let projects = ctx.state.projects.read().await;
    let configs = ctx.state.project_configs.read().await;
    let mut result = Vec::new();
    for (id, project) in projects.iter() {
        let mut entry = project.clone();
        if let Some(cfg) = configs.get(id) {
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("config".into(), cfg.clone());
                let name = cfg
                    .get("project_name")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .or_else(|| obj.get("name").and_then(|v| v.as_str()).map(String::from))
                    .unwrap_or_else(|| {
                        obj.get("path")
                            .and_then(|v| v.as_str())
                            .map(basename_from_path)
                            .unwrap_or_else(|| "Unknown".into())
                    });
                obj.insert("name".into(), json!(name));
            }
        } else if entry.get("name").is_none() {
            if let Some(obj) = entry.as_object_mut() {
                let name = obj
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(basename_from_path)
                    .unwrap_or_else(|| "Unknown".into());
                obj.insert("name".into(), json!(name));
            }
        }
        result.push(entry);
    }
    Json(result)
}

async fn create_project(
    State(ctx): State<ServerCtx>,
    Json(body): Json<ProjectCreate>,
) -> Result<Json<Value>, Response> {
    if !sandbox_ok(&body.sandbox_mode) {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            format!("sandbox_mode must be one of {SANDBOX_MODES:?}"),
        ));
    }
    let project_path = validate_project_path(&body.path)?;
    let project_id = Uuid::new_v4().to_string();
    let name = basename_from_path(&project_path);
    let project = json!({
        "id": project_id,
        "path": project_path,
        "name": name,
        "sandbox_mode": body.sandbox_mode,
        "state": "active",
    });
    let mut config = default_project_config(&project_path);
    if let Some(obj) = config.as_object_mut() {
        obj.insert("sandbox_mode".into(), json!(body.sandbox_mode));
    }
    ctx.state
        .projects
        .write()
        .await
        .insert(project_id.clone(), project.clone());
    ctx.state
        .project_configs
        .write()
        .await
        .insert(project_id.clone(), config.clone());
    ctx.state
        .project_last_active
        .write()
        .await
        .insert(project_id, monotonic_secs());
    if let Err(e) = local_config::write_local_config(&project_path, &config).await {
        warn!("failed to write .handover/config.yml on create: {e}");
    }
    persist_or_500(&ctx.state).await?;

    let mut out = project;
    if let Some(obj) = out.as_object_mut() {
        obj.insert("config".into(), config);
    }
    Ok(Json(out))
}

async fn merged_project_config(
    ctx: &ServerCtx,
    project_id: &str,
    path: &str,
) -> Result<Value, Response> {
    let mut configs = ctx.state.project_configs.write().await;
    if !configs.contains_key(project_id) {
        configs.insert(project_id.to_string(), default_project_config(path));
    }
    let persisted = configs[project_id].clone();
    drop(configs);

    let base = default_project_config(path);
    let file_config = local_config::read_local_config(path)
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut merged = base;
    if let Some(file) = file_config {
        merged = local_config::merge_config_values(merged, file);
    }
    merged = local_config::merge_config_values(merged, persisted);
    Ok(merged)
}

async fn get_project_config(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
) -> Result<Json<Value>, Response> {
    if !ctx.state.projects.read().await.contains_key(&project_id) {
        return Err(api_err(StatusCode::NOT_FOUND, "Project not found"));
    }
    let path = ctx
        .state
        .projects
        .read()
        .await
        .get(&project_id)
        .and_then(|p| p.get("path").and_then(|v| v.as_str()).map(String::from))
        .unwrap_or_default();

    Ok(Json(merged_project_config(&ctx, &project_id, &path).await?))
}

async fn save_project_config(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
    Json(body): Json<ProjectConfigBody>,
) -> Result<Json<Value>, Response> {
    let projects = ctx.state.projects.read().await;
    let project = projects
        .get(&project_id)
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Project not found"))?;
    let path = project
        .get("path")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_default();
    drop(projects);

    if !sandbox_ok(&body.sandbox_mode) {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            format!("sandbox_mode must be one of {SANDBOX_MODES:?}"),
        ));
    }
    if !HANDOFF_METHODS.contains(&body.handoff_method.as_str()) {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            format!("handoff_method must be one of {HANDOFF_METHODS:?}"),
        ));
    }
    if !HANDOFF_TEMPLATES.contains(&body.handoff_template.as_str()) {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            format!("handoff_template must be one of {HANDOFF_TEMPLATES:?}"),
        ));
    }

    let display_name = body
        .project_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| basename_from_path(&path));

    let saved = json!({
        "project_name": display_name,
        "sandbox_mode": body.sandbox_mode,
        "mem_limit": body.mem_limit,
        "handoff_method": body.handoff_method,
        "handoff_template": body.handoff_template,
        "custom_env_vars": body.custom_env_vars.unwrap_or_default(),
    });

    ctx.state
        .project_configs
        .write()
        .await
        .insert(project_id.clone(), saved.clone());
    if let Some(p) = ctx.state.projects.write().await.get_mut(&project_id) {
        if let Some(obj) = p.as_object_mut() {
            obj.insert("name".into(), json!(display_name));
        }
    }
    local_config::write_local_config(&path, &saved)
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    persist_or_500(&ctx.state).await?;
    Ok(Json(saved))
}

async fn activate_project(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
) -> Result<Json<Value>, Response> {
    let mut projects = ctx.state.projects.write().await;
    let project = projects
        .get_mut(&project_id)
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Project not found"))?;

    if project.get("state").and_then(|v| v.as_str()) == Some("suspended") {
        if let Ok(docker) = get_docker(&ctx).await {
            let ids = project_container_ids(&ctx.state, &project_id).await;
            for id in ids {
                if let Err(e) = docker.resume_container(&id).await {
                    warn!("failed to resume container while activating project: {e}");
                }
            }
        }
        if let Some(obj) = project.as_object_mut() {
            obj.insert("state".into(), json!("active"));
        }
    }
    drop(projects);

    *ctx.state.focused_project_id.write().await = Some(project_id.clone());
    ctx.state
        .project_last_active
        .write()
        .await
        .insert(project_id, monotonic_secs());
    persist_or_500(&ctx.state).await?;
    Ok(Json(json!({ "status": "ok" })))
}

async fn resume_project(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
) -> Result<Json<Value>, Response> {
    if !ctx.state.projects.read().await.contains_key(&project_id) {
        return Err(api_err(StatusCode::NOT_FOUND, "Project not found"));
    }
    let docker = get_docker(&ctx).await?;
    let ids = project_container_ids(&ctx.state, &project_id).await;
    for id in ids {
        docker
            .resume_container(&id)
            .await
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    if let Some(p) = ctx.state.projects.write().await.get_mut(&project_id) {
        if let Some(obj) = p.as_object_mut() {
            obj.insert("state".into(), json!("active"));
        }
    }
    ctx.state
        .project_last_active
        .write()
        .await
        .insert(project_id, monotonic_secs());
    persist_or_500(&ctx.state).await?;
    Ok(Json(json!({ "status": "ok", "state": "active" })))
}

async fn unload_project(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
) -> Json<Value> {
    if !ctx.state.projects.read().await.contains_key(&project_id) {
        return Json(json!({ "status": "ok", "message": "Already unloaded" }));
    }

    let instance_ids: Vec<String> = ctx
        .state
        .instances
        .read()
        .await
        .iter()
        .filter_map(|(id, inst)| {
            if inst.get("project_id").and_then(|v| v.as_str()) == Some(project_id.as_str()) {
                Some(id.clone())
            } else {
                None
            }
        })
        .collect();

    ctx.state.project_configs.write().await.remove(&project_id);
    ctx.state
        .project_last_active
        .write()
        .await
        .remove(&project_id);

    let docker = ctx.docker.read().await.clone();
    for instance_id in instance_ids {
        let instance = ctx.state.instances.write().await.remove(&instance_id);
        crate::pty::shutdown_instance_pty(&ctx.state, &instance_id).await;
        if let Some(inst) = instance {
            if let Some(cid) = inst.get("container_id").and_then(|v| v.as_str()) {
                if let Some(d) = docker.as_ref() {
                    if let Err(e) = d.stop_container(cid).await {
                        warn!("failed to stop container while unloading project: {e}");
                    }
                }
            }
        }
    }

    ctx.state.projects.write().await.remove(&project_id);
    if let Err(e) = ctx.state.save_persisted().await {
        warn!("failed to persist project unload: {e}");
    }
    Json(json!({ "status": "ok", "message": "Project unloaded" }))
}

async fn start_instance(
    State(ctx): State<ServerCtx>,
    Json(body): Json<InstanceStart>,
) -> Result<Json<Value>, Response> {
    let project = ctx
        .state
        .projects
        .read()
        .await
        .get(&body.project_id)
        .cloned()
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Project not found"))?;

    if !sandbox_ok(&body.sandbox_mode) {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            format!("sandbox_mode must be one of {SANDBOX_MODES:?}"),
        ));
    }

    let instance_id = Uuid::new_v4().to_string();
    let mut instance = json!({
        "id": instance_id,
        "project_id": body.project_id,
        "sandbox_mode": body.sandbox_mode,
    });

    if body.sandbox_mode == "native" {
        if let Some(obj) = instance.as_object_mut() {
            obj.insert("container_id".into(), Value::Null);
        }
    } else {
        let docker = get_docker(&ctx).await?;
        let path = project
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| api_err(StatusCode::INTERNAL_SERVER_ERROR, "project path missing"))?;
        let mem_limit = body.mem_limit.as_deref().unwrap_or(DEFAULT_MEM_LIMIT);
        let container_id = docker
            .start_container(path, &instance_id, mem_limit, body.custom_env_vars.as_ref())
            .await
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if let Some(obj) = instance.as_object_mut() {
            obj.insert("container_id".into(), json!(container_id));
            obj.insert("mem_limit".into(), json!(mem_limit));
        }
    }

    ctx.state
        .instances
        .write()
        .await
        .insert(instance_id.clone(), instance.clone());
    Ok(Json(instance))
}

async fn delete_instance(
    State(ctx): State<ServerCtx>,
    Path(instance_id): Path<String>,
) -> Json<Value> {
    let instance = ctx.state.instances.write().await.remove(&instance_id);
    if instance.is_none() {
        return Json(json!({ "status": "ok" }));
    }
    crate::pty::shutdown_instance_pty(&ctx.state, &instance_id).await;
    if let Some(inst) = instance {
        if let Some(cid) = inst.get("container_id").and_then(|v| v.as_str()) {
            if let Some(docker) = ctx.docker.read().await.clone() {
                let cid = cid.to_string();
                tokio::spawn(async move {
                    if let Err(e) = docker.stop_container(&cid).await {
                        warn!("failed to stop deleted instance container: {e}");
                    }
                });
            }
        }
    }
    Json(json!({ "status": "ok" }))
}

async fn instance_stats(
    State(ctx): State<ServerCtx>,
    Path(instance_id): Path<String>,
) -> Result<Json<Value>, Response> {
    let instance = ctx
        .state
        .instances
        .read()
        .await
        .get(&instance_id)
        .cloned()
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Instance not found"))?;

    let zeros = json!({
        "mem_used_mb": 0.0,
        "mem_limit_mb": 0.0,
        "cpu_percent": 0.0,
    });

    let container_id = match instance.get("container_id").and_then(|v| v.as_str()) {
        Some(id) if !id.is_empty() => id,
        _ => return Ok(Json(zeros)),
    };

    let docker = get_docker(&ctx).await?;
    Ok(Json(docker.get_container_stats(container_id).await))
}

async fn focus_instance(
    State(ctx): State<ServerCtx>,
    Path(instance_id): Path<String>,
) -> Result<Json<Value>, Response> {
    if !ctx.state.instances.read().await.contains_key(&instance_id) {
        return Err(api_err(StatusCode::NOT_FOUND, "Instance not found"));
    }
    Ok(Json(json!({ "status": "ok" })))
}

async fn send_prompt_to_instance(state: &AppState, instance_id: &str, prompt: &str) -> bool {
    if let Some(session) = state.pty_sessions.read().await.get(instance_id) {
        let _ = session.input_tx.send(b"\n".to_vec());
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let mut line = prompt.as_bytes().to_vec();
        line.push(b'\n');
        let _ = session.input_tx.send(line);
        return true;
    }
    false
}

async fn execute_handoff(
    State(ctx): State<ServerCtx>,
    Json(body): Json<HandoffRequest>,
) -> Result<Response, Response> {
    if !HANDOFF_METHODS.contains(&body.method.as_str()) {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            format!("method must be one of {HANDOFF_METHODS:?}"),
        ));
    }

    let project = ctx
        .state
        .projects
        .read()
        .await
        .get(&body.project_id)
        .cloned()
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Project not found"))?;

    let from_instance = ctx
        .state
        .instances
        .read()
        .await
        .get(&body.from_instance_id)
        .cloned()
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "from_instance not found"))?;
    let to_instance = ctx
        .state
        .instances
        .read()
        .await
        .get(&body.to_instance_id)
        .cloned()
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "to_instance not found"))?;

    if from_instance.get("project_id").and_then(|v| v.as_str()) != Some(body.project_id.as_str()) {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "from_instance does not belong to project_id",
        ));
    }
    if to_instance.get("project_id").and_then(|v| v.as_str()) != Some(body.project_id.as_str()) {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "to_instance does not belong to project_id",
        ));
    }

    let project_path = project
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| api_err(StatusCode::INTERNAL_SERVER_ERROR, "project path missing"))?;

    let mut handoff_message = "Handoff complete".to_string();
    let injection_prompt = if body.method == "git" {
        let outcome = match handoff::git_handoff(
            project_path,
            &format!("Handoff checkpoint: {}", body.task_description),
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(_) => {
                return Ok((
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "status": "error",
                        "message": "Project is not a Git repository. Initialize git first or use Summary handoff."
                    })),
                )
                    .into_response());
            }
        };
        match outcome {
            handoff::GitHandoffOutcome::Committed => {
                "You are taking over this project. The previous AI just committed their work. Please review the latest git commit using 'git log -1 -p' to understand the current state of the codebase. Your overall goal is: {task}. Continue working on this goal from where the previous AI left off."
            }
            handoff::GitHandoffOutcome::NoChanges => {
                handoff_message = "Handoff complete; no git changes needed a new commit".to_string();
                "You are taking over this project. The previous AI found no uncommitted changes to checkpoint. Please inspect the current worktree and recent history with 'git status' and 'git log -1 --stat'. Your overall goal is: {task}. Continue working on this goal from where the previous AI left off."
            }
        }
        .replace("{task}", &body.task_description)
    } else {
        let merged = merged_project_config(&ctx, &body.project_id, project_path).await?;
        let handoff_template = merged
            .get("handoff_template")
            .and_then(|v| v.as_str())
            .unwrap_or("generic");
        let previous_snapshot = handoff::latest_handoff_snapshot(project_path);
        let source_prompt =
            handoff::build_summary_source_prompt(&body.task_description, handoff_template);
        if !send_prompt_to_instance(&ctx.state, &body.from_instance_id, &source_prompt).await {
            return Err(api_err(
                StatusCode::BAD_REQUEST,
                "from_instance does not have an active terminal session",
            ));
        }

        let ready = handoff::wait_for_latest_handoff(
            project_path,
            previous_snapshot,
            handoff::HANDOFF_FILE_WAIT_TIMEOUT,
        )
        .await
        .map_err(|e| {
            api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed while waiting for handoff file: {e}"),
            )
        })?;

        if !ready {
            if let Err(e) = handoff::summary_handoff(project_path, &body.task_description).await {
                return Err(api_err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("failed to write fallback handoff file: {e}"),
                ));
            }
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "status": "error",
                    "message": "Source AI did not create a handoff file within 60 seconds. A fallback handoff file was written with only the goal you provided; review it before handing off manually."
                })),
            )
                .into_response());
        }

        handoff_message =
            "Source AI wrote the handoff file; target AI was directed to read it".to_string();
        format!(
            "You are taking over this project. Read '.handover/handoffs/latest.md' for the handoff from the previous AI. Your overall goal is: {}. Continue from that file.",
            body.task_description
        )
    };

    if !send_prompt_to_instance(&ctx.state, &body.to_instance_id, &injection_prompt).await {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "to_instance does not have an active terminal session",
        ));
    }

    Ok(Json(json!({ "status": "ok", "message": handoff_message })).into_response())
}

async fn project_path_for(ctx: &ServerCtx, project_id: &str) -> Result<String, Response> {
    ctx.state
        .projects
        .read()
        .await
        .get(project_id)
        .and_then(|p| p.get("path").and_then(|v| v.as_str()).map(String::from))
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Project not found"))
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
    if !ctx.state.projects.read().await.contains_key(&project_id) {
        return Err(api_err(StatusCode::NOT_FOUND, "Project not found"));
    }

    let instance_ids: Vec<String> = ctx
        .state
        .instances
        .read()
        .await
        .iter()
        .filter_map(|(id, inst)| {
            if inst.get("project_id").and_then(|v| v.as_str()) == Some(project_id.as_str()) {
                Some(id.clone())
            } else {
                None
            }
        })
        .collect();

    let total = instance_ids.len();
    let mut sent = 0u32;
    for id in instance_ids {
        if send_prompt_to_instance(&ctx.state, &id, prompt).await {
            sent += 1;
        }
    }

    Ok(Json(
        json!({ "status": "ok", "sent": sent, "total": total }),
    ))
}

async fn project_resources(
    State(ctx): State<ServerCtx>,
    Path(project_id): Path<String>,
) -> Result<Json<Value>, Response> {
    if !ctx.state.projects.read().await.contains_key(&project_id) {
        return Err(api_err(StatusCode::NOT_FOUND, "Project not found"));
    }

    use sysinfo::{MemoryRefreshKind, RefreshKind, System};
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()),
    );
    sys.refresh_memory();
    let total_mb = sys.total_memory() as f64 / (1024.0 * 1024.0);
    let used_mb = sys.used_memory() as f64 / (1024.0 * 1024.0);
    let ram_percent = if total_mb > 0.0 {
        used_mb / total_mb * 100.0
    } else {
        0.0
    };

    let instances_map = ctx.state.instances.read().await;
    let pty_sessions = ctx.state.pty_sessions.read().await;
    let project = ctx.state.projects.read().await.get(&project_id).cloned();
    let project_state = project
        .as_ref()
        .and_then(|p| p.get("state").and_then(|v| v.as_str()))
        .unwrap_or("active")
        .to_string();

    let project_instances: Vec<(String, Value)> = instances_map
        .iter()
        .filter_map(|(id, inst)| {
            if inst.get("project_id").and_then(|v| v.as_str()) == Some(project_id.as_str()) {
                Some((id.clone(), inst.clone()))
            } else {
                None
            }
        })
        .collect();
    drop(instances_map);
    drop(project);

    let docker = get_docker(&ctx).await.ok();
    let mut instance_rows = Vec::new();
    for (index, (instance_id, inst)) in project_instances.iter().enumerate() {
        let zeros = json!({
            "mem_used_mb": 0.0,
            "mem_limit_mb": 0.0,
            "cpu_percent": 0.0,
        });
        let stats = if let Some(d) = docker.as_ref() {
            if let Some(cid) = inst
                .get("container_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                d.get_container_stats(cid).await
            } else {
                zeros
            }
        } else {
            zeros
        };
        instance_rows.push(json!({
            "instance_id": instance_id,
            "label": format!("Terminal {}", index + 1),
            "sandbox_mode": inst.get("sandbox_mode"),
            "connected": pty_sessions.contains_key(instance_id),
            "stats": stats,
        }));
    }
    drop(pty_sessions);

    Ok(Json(json!({
        "system": {
            "ram_used_mb": used_mb,
            "ram_total_mb": total_mb,
            "ram_percent": ram_percent,
            "suspend_threshold_percent": RAM_SUSPEND_THRESHOLD,
            "emergency_threshold_percent": RAM_EMERGENCY_THRESHOLD,
        },
        "project": {
            "state": project_state,
            "instance_count": instance_rows.len(),
        },
        "instances": instance_rows,
    })))
}

async fn ws_pty(
    ws: WebSocketUpgrade,
    Path(instance_id): Path<String>,
    State(ctx): State<ServerCtx>,
) -> Result<impl IntoResponse, StatusCode> {
    if !ctx.state.instances.read().await.contains_key(&instance_id) {
        return Err(StatusCode::NOT_FOUND);
    }
    let state = ctx.state.clone();
    Ok(ws.on_upgrade(move |socket| pty::handle_pty_socket(socket, instance_id, state)))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();

    let state = AppState::new();
    if let Err(e) = state.load_persisted().await {
        warn!("[startup] failed to load persisted app state: {e}");
    }
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
    governor::spawn_governor(state.clone(), docker_slot.clone());

    let ctx = ServerCtx {
        state,
        docker: docker_slot,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(root))
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
        .with_state(ctx)
        .layer(cors);

    let addr: SocketAddr = format!("{}:{}", cli.host, cli.port).parse()?;
    tracing::debug!("handover-backend listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
