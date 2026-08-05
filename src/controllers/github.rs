use std::sync::Arc;

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;

use crate::error::AppError;
use crate::services::github;
use crate::state::AppState;

/// Proxy for the GitHub activity data the frontend displays, so browser-side calls
/// don't exhaust GitHub's unauthenticated rate limit. Cached server-side.
pub async fn get_activity(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let data = github::activity(&state.http, &state.config, &state.github_cache).await?;
    Ok(Json(data))
}

/// Contribution heatmap image. Served with `Cache-Control` so the browser caches it
/// instead of re-fetching on every tab switch / remount.
pub async fn get_heatmap(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    let (body, ct) = github::heatmap(&state.http, &state.config, &state.github_cache).await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(&ct).unwrap());
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=3600"));
    Ok((StatusCode::OK, headers, body).into_response())
}
