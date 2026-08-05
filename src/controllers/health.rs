use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use sqlx::Row;

use crate::state::AppState;

pub async fn get_health(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "status": "healthy",
        "timestamp": now_iso(),
        "uptime": uptime_secs(&state),
        "environment": state.config.app_env,
    }))
}

pub async fn get_detailed_health(State(state): State<Arc<AppState>>) -> Response {
    let start = std::time::Instant::now();

    let database = {
        let t = std::time::Instant::now();
        match sqlx::query("SELECT NOW()").execute(&state.db).await {
            Ok(_) => json!({
                "status": "healthy",
                "latency": t.elapsed().as_millis() as u64,
                "message": "Connected to Neon PostgreSQL",
            }),
            Err(e) => json!({ "status": "unhealthy", "error": e.to_string() }),
        }
    };

    let r2_client = {
        let t = std::time::Instant::now();
        match state.r2.list_buckets().send().await {
            Ok(out) => json!({
                "status": "healthy",
                "latency": t.elapsed().as_millis() as u64,
                "buckets": out.buckets().len(),
                "message": "Connected to r2Client Storage",
            }),
            Err(e) => json!({ "status": "unhealthy", "error": e.to_string() }),
        }
    };

    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let total = sys.total_memory();
    let free = sys.free_memory();
    let used = total.saturating_sub(free);
    let usage = if total == 0 { 0.0 } else { used as f64 * 100.0 / total as f64 };

    let pid = sysinfo::Pid::from_u32(std::process::id());
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), false);
    let proc_mem = sys.process(pid).map(|p| p.memory()).unwrap_or(0);

    let memory = json!({
        "status": if usage < 90.0 { "healthy" } else { "warning" },
        "total": gb(total),
        "used": gb(used),
        "free": gb(free),
        "usage": format!("{usage:.2}%"),
        // Node reported the V8 heap; Rust has none, report process RSS instead.
        "process": { "rss": mb(proc_mem) },
    });

    let checks = json!({
        "database": database,
        "r2Client": r2_client,
        "memory": memory,
        // parity: the Node handler never populated `disk`
        "disk": { "status": "unknown" },
    });

    let all_healthy = ["database", "r2Client", "memory", "disk"]
        .iter()
        .all(|k| is_ok(&checks[k]));

    let response = json!({
        "status": if all_healthy { "healthy" } else { "degraded" },
        "timestamp": now_iso(),
        "uptime": format_uptime(uptime_secs(&state) as u64),
        "uptimeSeconds": uptime_secs(&state) as u64,
        "environment": state.config.app_env,
        "version": env!("CARGO_PKG_VERSION"),
        "checks": checks,
        "responseTime": format!("{}ms", start.elapsed().as_millis()),
        "system": {
            "platform": map_os(std::env::consts::OS),
            "arch": map_arch(std::env::consts::ARCH),
            "rustVersion": rustc_version_runtime::version().to_string(),
            "cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
            "hostname": gethostname::gethostname().to_string_lossy().into_owned(),
            "loadAverage": load_average(),
        },
    });

    let status = if all_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(response)).into_response()
}

pub async fn ping() -> Json<Value> {
    Json(json!({
        "pong": true,
        "timestamp": chrono::Utc::now().timestamp_millis(),
    }))
}

pub async fn get_database_health(State(state): State<Arc<AppState>>) -> Response {
    let t = std::time::Instant::now();
    match sqlx::query("SELECT NOW() as time, version() as version")
        .fetch_one(&state.db)
        .await
    {
        Ok(row) => {
            let latency = t.elapsed().as_millis() as u64;
            let time: chrono::DateTime<chrono::Utc> = row.get("time");
            let version: String = row.get("version");
            let short = version.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
            let body = json!({
                "status": "healthy",
                "latency": format!("{latency}ms"),
                "timestamp": time.to_rfc3339(),
                "version": short,
                "pool": {
                    "total": state.db.size(),
                    "idle": state.db.num_idle(),
                    // sqlx exposes no waiting counter (pg's `waitingCount` has no equivalent)
                    "waiting": 0,
                },
            });
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "unhealthy", "error": e.to_string() })),
        )
            .into_response(),
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn uptime_secs(state: &AppState) -> f64 {
    state.started.elapsed().as_secs_f64()
}

/// Port of `formatUptime` in healthController.js.
fn format_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let minutes = (secs % 3_600) / 60;
    let seconds = secs % 60;
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    parts.push(format!("{seconds}s"));
    parts.join(" ")
}

fn is_ok(check: &Value) -> bool {
    matches!(
        check.get("status").and_then(|s| s.as_str()),
        Some("healthy") | Some("warning")
    )
}

fn gb(bytes: u64) -> String {
    format!("{:.2}", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
}

fn mb(bytes: u64) -> String {
    format!("{:.2}", bytes as f64 / 1024.0 / 1024.0)
}

fn map_os(os: &str) -> &str {
    match os {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    }
}

fn map_arch(arch: &str) -> &str {
    match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }
}

fn load_average() -> Vec<String> {
    #[cfg(unix)]
    {
        let mut avg = [0.0f64; 3];
        let n = unsafe { libc::getloadavg(avg.as_mut_ptr(), 3) };
        if n > 0 {
            avg[..n as usize].iter().map(|x| format!("{x:.2}")).collect()
        } else {
            vec!["0.00".to_string(); 3]
        }
    }
    #[cfg(not(unix))]
    {
        vec!["0.00".to_string(); 3]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_formatting_matches_node() {
        assert_eq!(format_uptime(0), "0s");
        assert_eq!(format_uptime(59), "59s");
        assert_eq!(format_uptime(60), "1m 0s");
        assert_eq!(format_uptime(3661), "1h 1m 1s");
        assert_eq!(format_uptime(90_061), "1d 1h 1m 1s");
    }

    #[test]
    fn check_status_evaluation() {
        assert!(is_ok(&json!({ "status": "healthy" })));
        assert!(is_ok(&json!({ "status": "warning" })));
        assert!(!is_ok(&json!({ "status": "unhealthy" })));
        assert!(!is_ok(&json!({ "status": "unknown" })));
    }
}
