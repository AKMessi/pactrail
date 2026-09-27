//! Loopback-only browser front end. Every action goes through the same CLI
//! contract as the terminal, preserving its receipt and transaction checks.
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    workspace: PathBuf,
    state_dir: Option<PathBuf>,
    executable: PathBuf,
    port: u16,
    jobs: Arc<Mutex<HashMap<String, Job>>>,
    read_lock: Arc<Mutex<()>>,
}

struct Job {
    state: &'static str,
    goal: String,
    output: Option<Value>,
    error: Option<String>,
    cancel: Option<oneshot::Sender<()>>,
}

#[derive(Serialize)]
struct JobView {
    id: String,
    state: &'static str,
    goal: String,
    output: Option<Value>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct RunRequest {
    goal: String,
    provider: String,
    model: String,
    base_url: Option<String>,
    api_key_env: Option<String>,
    max_turns: Option<u16>,
    process_backend: Option<String>,
    sandbox_image: Option<String>,
    max_cost_microusd: Option<u64>,
    request_timeout_seconds: Option<u64>,
    apply: Option<bool>,
}

type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;

pub async fn serve(
    workspace: &FsPath,
    state_dir: Option<&FsPath>,
    port: u16,
) -> Result<(), String> {
    if port == 0 {
        return Err("choose a port from 1 to 65535".to_owned());
    }
    let workspace = workspace
        .canonicalize()
        .map_err(|e| format!("workspace: {e}"))?;
    if !workspace.is_dir() {
        return Err("workspace must be a directory".to_owned());
    }
    // dotenvy leaves already-exported variables intact. Only this workspace's
    // .env is loaded; secrets never enter an HTTP response.
    let _ = dotenvy::from_path(workspace.join(".env"));
    let state = AppState {
        workspace,
        state_dir: state_dir.map(FsPath::to_path_buf),
        executable: std::env::current_exe().map_err(|e| e.to_string())?,
        port,
        jobs: Arc::new(Mutex::new(HashMap::new())),
        read_lock: Arc::new(Mutex::new(())),
    };
    let router = router(state);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|e| format!("could not bind loopback port {port}: {e}"))?;
    let actual_port = listener.local_addr().map_err(|e| e.to_string())?.port();
    crate::output::write_human_stdout(&format!("Pactrail web: http://127.0.0.1:{actual_port}\n"))
        .map_err(|e| e.to_string())?;
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| e.to_string())
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/settings", get(index))
        .route("/runs/{id}", get(index))
        .route("/app.css", get(styles))
        .route("/app.js", get(script))
        .route("/icons.svg", get(icons))
        .route("/fonts/martian-mono-latin.woff2", get(martian_font))
        .route("/fonts/spline-sans-latin.woff2", get(spline_font))
        .route("/fonts/spline-sans-mono-latin.woff2", get(spline_mono_font))
        .route("/api/bootstrap", get(bootstrap))
        .route("/api/runs", get(runs).post(start_run))
        .route("/api/runs/{id}/events", get(run_events))
        .route(
            "/api/runs/{id}/{operation}",
            get(run_detail).post(run_action),
        )
        .route("/api/jobs", get(jobs))
        .route("/api/jobs/{id}/cancel", post(cancel_job))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            local_request_guard,
        ))
        .with_state(state)
}

async fn local_request_guard(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok());
    let allowed_hosts = [
        format!("127.0.0.1:{}", state.port),
        format!("localhost:{}", state.port),
    ];
    if !host.is_some_and(|h| allowed_hosts.iter().any(|allowed| h == allowed)) {
        return (StatusCode::FORBIDDEN, "Invalid host").into_response();
    }
    if request.method() != axum::http::Method::GET && request.method() != axum::http::Method::HEAD {
        let origin = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|h| h.to_str().ok());
        if !origin.is_some_and(|o| {
            o == format!("http://127.0.0.1:{}", state.port)
                || o == format!("http://localhost:{}", state.port)
        }) {
            return (StatusCode::FORBIDDEN, "Invalid origin").into_response();
        }
    }
    next.run(request).await
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    asset_bytes(content_type, body.as_bytes())
}

fn asset_bytes(content_type: &'static str, body: &'static [u8]) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; font-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"));
    (headers, body).into_response()
}

async fn index() -> Response {
    asset(
        "text/html; charset=utf-8",
        include_str!("../web/index.html"),
    )
}
async fn styles() -> Response {
    asset("text/css; charset=utf-8", include_str!("../web/app.css"))
}
async fn script() -> Response {
    asset(
        "text/javascript; charset=utf-8",
        include_str!("../web/app.js"),
    )
}
async fn icons() -> Response {
    asset("image/svg+xml", include_str!("../web/icons.svg"))
}
async fn martian_font() -> Response {
    asset_bytes(
        "font/woff2",
        include_bytes!("../web/fonts/martian-mono-latin.woff2"),
    )
}
async fn spline_font() -> Response {
    asset_bytes(
        "font/woff2",
        include_bytes!("../web/fonts/spline-sans-latin.woff2"),
    )
}
async fn spline_mono_font() -> Response {
    asset_bytes(
        "font/woff2",
        include_bytes!("../web/fonts/spline-sans-mono-latin.woff2"),
    )
}

async fn run_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>>>,
    (StatusCode, Json<Value>),
> {
    valid_id(&id)?;
    let stream = futures_util::stream::unfold(
        (
            state,
            id,
            0usize,
            tokio::time::interval(std::time::Duration::from_millis(1800)),
        ),
        |(state, id, mut seen, mut interval)| async move {
            loop {
                interval.tick().await;
                let read_guard = state.read_lock.lock().await;
                let state_copy = state.clone();
                let id_copy = id.clone();
                let events = tokio::task::spawn_blocking(move || {
                    let root = crate::commands::state_dir(
                        &state_copy.workspace,
                        state_copy.state_dir.as_deref(),
                    )
                    .ok()?;
                    let run_id = crate::commands::parse_run_id(&id_copy).ok()?;
                    crate::commands::load_trace(&root, run_id).ok()
                })
                .await
                .ok()
                .flatten();
                drop(read_guard);
                if let Some(events) = events
                    && events.len() > seen
                {
                    let fresh = &events[seen..];
                    seen = events.len();
                    let payload = serde_json::to_string(fresh).unwrap_or_default();
                    return Some((
                        Ok(Event::default().event("trace").data(payload)),
                        (state, id, seen, interval),
                    ));
                }
            }
        },
    );
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn cli_json(state: &AppState, args: &[&str]) -> ApiResult {
    let _read_guard = state.read_lock.lock().await;
    let mut command = Command::new(&state.executable);
    command.arg("--workspace").arg(&state.workspace);
    if let Some(dir) = &state.state_dir {
        command.arg("--state-dir").arg(dir);
    }
    let output = command
        .args(args)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(internal)?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": message}))));
    }
    serde_json::from_slice::<Value>(&output.stdout)
        .map(Json)
        .map_err(internal)
}

async fn bootstrap(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "workspace": state.workspace,
        "version": env!("CARGO_PKG_VERSION"),
        "providers": ["ollama", "open-ai-compatible", "open-ai", "open-ai-responses", "anthropic", "gemini"],
        "defaults": {
            "provider": if std::env::var_os("OPENROUTER_API_KEY").is_some() { "open-ai-compatible" } else { "ollama" },
            "model": std::env::var("PACTRAIL_MODEL").unwrap_or_else(|_| if std::env::var_os("OPENROUTER_API_KEY").is_some() { "stealth/space-bunny-alpha".to_owned() } else { String::new() }),
            "base_url": if std::env::var_os("OPENROUTER_API_KEY").is_some() { "https://openrouter.ai/api/v1" } else { "" },
            "api_key_env": if std::env::var_os("OPENROUTER_API_KEY").is_some() { "OPENROUTER_API_KEY" } else { "OPENAI_API_KEY" }
        }
    }))
}

async fn runs(State(state): State<AppState>) -> ApiResult {
    cli_json(&state, &["list", "--json"]).await
}

async fn run_detail(
    State(state): State<AppState>,
    Path((id, kind)): Path<(String, String)>,
) -> ApiResult {
    valid_id(&id)?;
    if !["inspect", "diff", "trace"].contains(&kind.as_str()) {
        return Err(bad("Unknown detail"));
    }
    let Json(mut value) = cli_json(&state, &[&kind, &id, "--json"]).await?;
    if kind == "inspect"
        && let Some(result) = read_web_result(&state, &id)
    {
        value["web_result"] = result;
    }
    Ok(Json(value))
}

fn web_result_dir(state: &AppState) -> Option<PathBuf> {
    crate::commands::state_dir(&state.workspace, state.state_dir.as_deref())
        .ok()
        .map(|root| root.join("web-results"))
}

fn read_web_result(state: &AppState, id: &str) -> Option<Value> {
    let path = web_result_dir(state)?.join(format!("{id}.json"));
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 300_000 {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn save_web_result(state: &AppState, value: &Value) -> Result<(), String> {
    let id = value["run_id"].as_str().ok_or("run result has no ID")?;
    Uuid::parse_str(id).map_err(|e| e.to_string())?;
    let directory = web_result_dir(state).ok_or("invalid state directory")?;
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let metadata = std::fs::symlink_metadata(&directory).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("web result directory is not a real local directory".to_owned());
    }
    let summary = value["summary"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(100_000)
        .collect::<String>();
    let result = json!({
        "summary": summary,
        "tokens": value["tokens"],
        "cost_microusd": value["cost_microusd"],
    });
    let mut temp = tempfile::NamedTempFile::new_in(&directory).map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut temp, &result).map_err(|e| e.to_string())?;
    temp.flush().map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(directory.join(format!("{id}.json")))
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn run_action(
    State(state): State<AppState>,
    Path((id, action)): Path<(String, String)>,
) -> ApiResult {
    valid_id(&id)?;
    if !["apply", "discard", "resume"].contains(&action.as_str()) {
        return Err(bad("Unknown action"));
    }
    if action == "resume" {
        return resume_run(state, id).await;
    }
    cli_json(&state, &[&action, &id, "--json"]).await
}

async fn resume_run(state: AppState, run_id: String) -> ApiResult {
    let mut jobs = state.jobs.lock().await;
    if jobs
        .values()
        .any(|job| job.state == "running" || job.state == "cancelling")
    {
        return Err(bad("A run is already active in this workspace"));
    }
    let id = Uuid::now_v7().to_string();
    let (cancel_tx, cancel_rx) = oneshot::channel();
    jobs.insert(
        id.clone(),
        Job {
            state: "running",
            goal: format!("Resume {run_id}"),
            output: None,
            error: None,
            cancel: Some(cancel_tx),
        },
    );
    drop(jobs);
    let job_id = id.clone();
    tokio::spawn(async move {
        let result = execute_resume(&state, &run_id, cancel_rx).await;
        finish_job(&state, &job_id, result).await;
    });
    Ok(Json(json!({"id": id, "state": "running"})))
}

async fn jobs(State(state): State<AppState>) -> Json<Value> {
    let jobs = state.jobs.lock().await;
    Json(json!(
        jobs.iter()
            .map(|(id, job)| JobView {
                id: id.clone(),
                state: job.state,
                goal: job.goal.clone(),
                output: job.output.clone(),
                error: job.error.clone()
            })
            .collect::<Vec<_>>()
    ))
}

async fn start_run(State(state): State<AppState>, Json(request): Json<RunRequest>) -> ApiResult {
    let goal = request.goal.trim();
    let model = request.model.trim();
    if goal.is_empty() || goal.len() > 16_000 {
        return Err(bad("Enter a task under 16,000 characters"));
    }
    if model.is_empty() || model.len() > 200 || model.starts_with('-') {
        return Err(bad("Choose a model"));
    }
    if ![
        "ollama",
        "open-ai",
        "open-ai-responses",
        "open-ai-compatible",
        "anthropic",
        "gemini",
    ]
    .contains(&request.provider.as_str())
    {
        return Err(bad("Unknown provider"));
    }
    if !["disabled", "oci", "native"]
        .contains(&request.process_backend.as_deref().unwrap_or("disabled"))
    {
        return Err(bad("Unknown process backend"));
    }
    if request.process_backend.as_deref() == Some("oci")
        && !request.sandbox_image.as_ref().is_some_and(|image| {
            !image.trim().is_empty() && image.len() <= 500 && !image.starts_with('-')
        })
    {
        return Err(bad("OCI process execution requires a local sandbox image"));
    }
    if request.max_turns.is_some_and(|n| n == 0 || n > 200) {
        return Err(bad("Turns must be between 1 and 200"));
    }
    if request
        .request_timeout_seconds
        .is_some_and(|n| !(1..=3600).contains(&n))
    {
        return Err(bad("Request timeout must be between 1 and 3600 seconds"));
    }
    if request
        .base_url
        .as_ref()
        .is_some_and(|u| u.len() > 2048 || !(u.starts_with("http://") || u.starts_with("https://")))
    {
        return Err(bad("Enter an HTTP or HTTPS endpoint"));
    }
    if request.api_key_env.as_ref().is_some_and(|name| {
        name.len() > 100 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }) {
        return Err(bad("Invalid key environment variable name"));
    }
    let mut jobs = state.jobs.lock().await;
    if jobs
        .values()
        .any(|job| job.state == "running" || job.state == "cancelling")
    {
        return Err(bad("A run is already active in this workspace"));
    }
    let id = Uuid::now_v7().to_string();
    let (cancel_tx, cancel_rx) = oneshot::channel();
    jobs.insert(
        id.clone(),
        Job {
            state: "running",
            goal: goal.to_owned(),
            output: None,
            error: None,
            cancel: Some(cancel_tx),
        },
    );
    drop(jobs);
    let state_copy = state.clone();
    let id_copy = id.clone();
    tokio::spawn(async move {
        let result = execute_run(&state_copy, request, cancel_rx).await;
        finish_job(&state_copy, &id_copy, result).await;
    });
    Ok(Json(json!({"id": id, "state": "running"})))
}

async fn finish_job(state: &AppState, id: &str, result: Result<Value, String>) {
    let mut jobs = state.jobs.lock().await;
    if let Some(job) = jobs.get_mut(id) {
        job.cancel = None;
        match result {
            Ok(value) => {
                job.state = if value["outcome"] == "cancelled" {
                    "cancelled"
                } else {
                    "complete"
                };
                job.output = Some(value);
            }
            Err(error) if error == "Run cancelled" => {
                job.state = "cancelled";
                job.error = Some(error);
            }
            Err(error) => {
                job.state = "failed";
                job.error = Some(error);
            }
        }
    }
    if jobs.len() > 50 {
        let mut finished = jobs
            .iter()
            .filter(|(_, job)| job.state != "running")
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        finished.sort_unstable();
        for old in finished.into_iter().take(jobs.len() - 50) {
            jobs.remove(&old);
        }
    }
}

async fn execute_run(
    state: &AppState,
    request: RunRequest,
    cancel: oneshot::Receiver<()>,
) -> Result<Value, String> {
    let mut command = Command::new(&state.executable);
    command.arg("--workspace").arg(&state.workspace);
    if let Some(dir) = &state.state_dir {
        command.arg("--state-dir").arg(dir);
    }
    command.args([
        "run",
        "--provider",
        &request.provider,
        "--model",
        &request.model,
        "--output",
        "json",
        "--max-turns",
        &request.max_turns.unwrap_or(24).to_string(),
    ]);
    command.args([
        "--process-backend",
        request.process_backend.as_deref().unwrap_or("disabled"),
    ]);
    if request
        .process_backend
        .as_deref()
        .is_some_and(|backend| backend != "disabled")
    {
        command.args(["--process-approval", "allow-run"]);
    }
    if let Some(image) = &request.sandbox_image
        && request.process_backend.as_deref() == Some("oci")
    {
        command.args(["--sandbox-image", image]);
    }
    if let Some(url) = &request.base_url
        && !url.is_empty()
    {
        command.args(["--base-url", url]);
    }
    if let Some(name) = &request.api_key_env {
        command.args(["--api-key-env", name]);
    }
    if request.apply.unwrap_or(false) {
        command.arg("--apply");
    }
    if let Some(cost) = request.max_cost_microusd
        && cost > 0
    {
        command.args(["--max-cost-microusd", &cost.to_string()]);
    }
    if let Some(seconds) = request.request_timeout_seconds {
        command.args(["--request-timeout-seconds", &seconds.to_string()]);
    }
    command.arg("--").arg(request.goal);
    let value = run_child(command, cancel).await?;
    if let Err(error) = save_web_result(state, &value) {
        tracing::warn!("could not persist web run summary: {error}");
    }
    Ok(value)
}

async fn execute_resume(
    state: &AppState,
    run_id: &str,
    cancel: oneshot::Receiver<()>,
) -> Result<Value, String> {
    let mut command = Command::new(&state.executable);
    command.arg("--workspace").arg(&state.workspace);
    if let Some(dir) = &state.state_dir {
        command.arg("--state-dir").arg(dir);
    }
    command.args(["resume", run_id, "--output", "json"]);
    let value = run_child(command, cancel).await?;
    if let Err(error) = save_web_result(state, &value) {
        tracing::warn!("could not persist web run summary: {error}");
    }
    Ok(value)
}

async fn run_child(
    mut command: Command,
    mut cancel: oneshot::Receiver<()>,
) -> Result<Value, String> {
    command.kill_on_drop(true);
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let child = command.spawn().map_err(|e| e.to_string())?;
    let child_id = child.id();
    let mut completion = Box::pin(child.wait_with_output());
    let output = tokio::select! {
        result = &mut completion => result.map_err(|e| e.to_string())?,
        _ = &mut cancel => {
            #[cfg(unix)]
            if let Some(id) = child_id {
                // The CLI translates SIGINT into a cooperative cancellation
                // event and writes an integrity-checked partial receipt.
                nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(
                        i32::try_from(id).map_err(|_| "Run process ID is invalid")?
                    ),
                    nix::sys::signal::Signal::SIGINT,
                )
                .map_err(|error| format!("Could not signal run: {error}"))?;
            }
            #[cfg(not(unix))]
            return Err("Run cancelled".to_owned());
            tokio::time::timeout(std::time::Duration::from_mins(1), &mut completion)
                .await
                .map_err(|_| "Run did not stop within 60 seconds".to_owned())?
                .map_err(|e| e.to_string())?
        },
    };
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("Invalid run result: {e}"))
}

async fn cancel_job(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let mut jobs = state.jobs.lock().await;
    let job = jobs.get_mut(&id).ok_or_else(|| bad("Job not found"))?;
    if let Some(cancel) = job.cancel.take() {
        let _ = cancel.send(());
        job.state = "cancelling";
    }
    Ok(Json(json!({"id": id, "state": "cancelling"})))
}

fn valid_id(id: &str) -> Result<(), (StatusCode, Json<Value>)> {
    Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| bad("Invalid run ID"))
}
fn bad(message: &str) -> (StatusCode, Json<Value>) {
    (StatusCode::BAD_REQUEST, Json(json!({"error": message})))
}
fn internal(error: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": error.to_string()})),
    )
}

#[cfg(test)]
mod tests {
    use super::{AppState, read_web_result, router, save_web_result};
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tower::ServiceExt;

    #[tokio::test]
    async fn loopback_ui_rejects_host_and_cross_origin_requests()
    -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempfile::tempdir()?;
        let app = router(AppState {
            workspace: workspace.path().to_path_buf(),
            state_dir: None,
            executable: std::env::current_exe()?,
            port: 4173,
            jobs: Arc::new(Mutex::new(HashMap::new())),
            read_lock: Arc::new(Mutex::new(())),
        });

        let bad_host = Request::builder()
            .uri("/api/bootstrap")
            .header(header::HOST, "evil.example")
            .body(Body::empty())?;
        let response = app.clone().oneshot(bad_host).await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let bad_origin = Request::builder()
            .method("POST")
            .uri("/api/runs")
            .header(header::HOST, "127.0.0.1:4173")
            .header(header::ORIGIN, "http://evil.example")
            .body(Body::empty())?;
        let response = app.clone().oneshot(bad_origin).await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let same_origin = Request::builder()
            .method("POST")
            .uri("/api/runs")
            .header(header::HOST, "127.0.0.1:4173")
            .header(header::ORIGIN, "http://127.0.0.1:4173")
            .body(Body::empty())?;
        let response = app.oneshot(same_origin).await?;
        assert_ne!(response.status(), StatusCode::FORBIDDEN);
        Ok(())
    }

    #[test]
    fn browser_summary_survives_a_new_server_state() -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempfile::tempdir()?;
        let state = AppState {
            workspace: workspace.path().to_path_buf(),
            state_dir: None,
            executable: std::env::current_exe()?,
            port: 4173,
            jobs: Arc::new(Mutex::new(HashMap::new())),
            read_lock: Arc::new(Mutex::new(())),
        };
        let id = uuid::Uuid::now_v7().to_string();
        save_web_result(
            &state,
            &json!({"run_id": id, "summary": "Review this separately from evidence", "tokens": 42, "cost_microusd": null}),
        )?;
        assert_eq!(
            read_web_result(&state, &id).and_then(|result| result["tokens"].as_u64()),
            Some(42)
        );
        Ok(())
    }
}
