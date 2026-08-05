use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::state::AppState;

/// Simple fixed-window in-memory rate limiter per key (IP).
///
/// Deliberately hand-rolled instead of tower-governor: the auth limiter needs
/// "skip successful requests" (express-rate-limit semantics) which tower-governor
/// doesn't support. ponytail: in-memory, single instance; swap to a shared store
/// if deployed multi-instance.
pub struct RateLimiter {
    window: Duration,
    max: usize,
    counters: Mutex<HashMap<String, (Instant, usize)>>,
}

impl RateLimiter {
    pub fn new(window: Duration, max: usize) -> Self {
        Self {
            window,
            max,
            counters: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn check(&self, key: &str) -> bool {
        let mut c = self.counters.lock().unwrap();
        let now = Instant::now();
        let (start, count) = c.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(*start) > self.window {
            *start = now;
            *count = 0;
        }
        *count < self.max
    }

    pub(crate) fn record(&self, key: &str) {
        let mut c = self.counters.lock().unwrap();
        let now = Instant::now();
        let (start, count) = c.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(*start) > self.window {
            *start = now;
            *count = 0;
        }
        *count += 1;
    }
}

/// The three global limiters, mirroring middleware/rateLimiter.js.
pub struct RateLimiters {
    pub api: RateLimiter,
    pub auth: RateLimiter,
    pub upload: RateLimiter,
    pub chat: RateLimiter,
}

impl RateLimiters {
    pub fn new() -> Self {
        Self {
            api: RateLimiter::new(Duration::from_secs(60), 300),
            auth: RateLimiter::new(Duration::from_secs(15 * 60), 5),
            upload: RateLimiter::new(Duration::from_secs(60 * 60), 50),
            chat: RateLimiter::new(Duration::from_secs(60 * 60), 50),
        }
    }
}

/// apiLimiter — applies to every /api/* request.
pub async fn api_limit(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let key = client_ip(&req);
    let limiter = &state.limiters.api;
    if !limiter.check(&key) {
        return rate_limit_response("Too many requests, please try again later");
    }
    let resp = next.run(req).await;
    limiter.record(&key);
    resp
}

/// authLimiter — login/register only; counts only successful requests.
pub async fn auth_limit(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let key = client_ip(&req);
    let limiter = &state.limiters.auth;
    if !limiter.check(&key) {
        return rate_limit_response("Too many login attempts, please try again after 15 minutes");
    }
    let resp = next.run(req).await;
    if resp.status().is_success() {
        limiter.record(&key);
    }
    resp
}

/// uploadLimiter — applied on /api/upload routes (Phase 5).
pub async fn upload_limit(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let key = client_ip(&req);
    let limiter = &state.limiters.upload;
    if !limiter.check(&key) {
        return rate_limit_response("Upload limit reached, please try again later");
    }
    let resp = next.run(req).await;
    limiter.record(&key);
    resp
}

/// Equivalent of Express `trust proxy = 1`: prefer first X-Forwarded-For, else socket addr.
pub fn client_ip(req: &Request) -> String {
    let addr = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0);
    client_ip_from(req.headers(), addr)
}

/// Same as `client_ip` but built from already-extracted parts (used in handlers
/// that take `HeaderMap` + `ConnectInfo` directly).
pub fn client_ip_from(headers: &axum::http::HeaderMap, addr: Option<SocketAddr>) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            let ip = first.trim();
            if !ip.is_empty() {
                return ip.to_string();
            }
        }
    }
    addr.map(|a| a.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn rate_limit_response(message: &str) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({ "success": false, "message": message })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_within_window() {
        let l = RateLimiter::new(Duration::from_secs(60), 2);
        assert!(l.check("1.2.3.4"));
        l.record("1.2.3.4");
        assert!(l.check("1.2.3.4"));
        l.record("1.2.3.4");
        assert!(!l.check("1.2.3.4")); // 2/2 reached
        assert!(l.check("5.6.7.8")); // different key unaffected
    }

    #[test]
    fn window_expires() {
        let l = RateLimiter::new(Duration::from_millis(5), 1);
        l.record("k");
        assert!(!l.check("k"));
        std::thread::sleep(Duration::from_millis(20));
        assert!(l.check("k")); // window reset
    }
}
