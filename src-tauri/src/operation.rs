use serde_json::json;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const PROFILE_LOCK_STRIPES: usize = 64;
const MAX_DISCOVERY_GENERATIONS: usize = 128;
const DISCOVERY_GENERATION_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_OPERATION_LOG_BYTES: u64 = 2 * 1024 * 1024;

static PROFILE_LOCKS: OnceLock<Vec<Mutex<()>>> = OnceLock::new();
static DISCOVERY_GENERATIONS: OnceLock<Mutex<HashMap<String, (u64, Instant)>>> = OnceLock::new();
static OPERATION_LOG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn lock_profile(profile_id: &str) -> Result<MutexGuard<'static, ()>, String> {
    let locks = PROFILE_LOCKS.get_or_init(|| {
        (0..PROFILE_LOCK_STRIPES)
            .map(|_| Mutex::new(()))
            .collect::<Vec<_>>()
    });
    let mut hasher = DefaultHasher::new();
    profile_id.hash(&mut hasher);
    let index = hasher.finish() as usize % locks.len();
    locks[index]
        .lock()
        .map_err(|_| "Profile operation lock is unavailable.".to_string())
}

pub fn begin_discovery(profile_id: &str, request_id: u64) -> Result<(), String> {
    let mut generations = DISCOVERY_GENERATIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "Discovery request coordinator is unavailable.".to_string())?;
    let now = Instant::now();
    generations.retain(|_, (_, touched)| now.duration_since(*touched) <= DISCOVERY_GENERATION_TTL);
    if generations.len() >= MAX_DISCOVERY_GENERATIONS && !generations.contains_key(profile_id) {
        if let Some(oldest) = generations
            .iter()
            .min_by_key(|(_, (_, touched))| *touched)
            .map(|(key, _)| key.clone())
        {
            generations.remove(&oldest);
        }
    }
    generations.insert(profile_id.to_string(), (request_id, now));
    Ok(())
}

pub fn discovery_is_current(profile_id: &str, request_id: u64) -> bool {
    DISCOVERY_GENERATIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|generations| generations.get(profile_id).copied())
        .is_some_and(|(current, _)| current == request_id)
}

pub async fn run_blocking<T, F>(
    app: AppHandle,
    operation: &'static str,
    profile_id: Option<String>,
    task: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let started = Instant::now();
    let result = tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| format!("{operation} stopped unexpectedly: {error}"))?;
    let elapsed = started.elapsed();
    let error = result.as_ref().err().cloned();
    if error.is_some() || elapsed >= Duration::from_millis(250) {
        record_operation(
            &app,
            operation,
            profile_id.as_deref(),
            elapsed,
            error.as_deref(),
        );
    }
    result
}

pub fn diagnostics_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|root| root.join("diagnostics").join("operations.jsonl"))
        .map_err(|error| format!("Could not resolve the diagnostics folder: {error}"))
}

fn record_operation(
    app: &AppHandle,
    operation: &str,
    profile_id: Option<&str>,
    elapsed: Duration,
    error: Option<&str>,
) {
    let Ok(_guard) = OPERATION_LOG_LOCK.get_or_init(|| Mutex::new(())).lock() else {
        return;
    };
    let Ok(path) = diagnostics_path(app) else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    if path
        .metadata()
        .is_ok_and(|metadata| metadata.len() >= MAX_OPERATION_LOG_BYTES)
    {
        let previous = parent.join("operations.previous.jsonl");
        let _ = fs::remove_file(&previous);
        let _ = fs::rename(&path, previous);
    }

    let entry = json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "operation": operation,
        "profileId": profile_id,
        "durationMs": elapsed.as_millis(),
        "status": if error.is_some() { "error" } else { "completed" },
        "error": error,
    });
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{entry}");
    }
}
