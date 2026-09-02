use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::middleware::auth::AdminAuth;
use crate::services::rag;
use crate::state::AppState;

/// Admin-only view of the RAG index: chunk counts by type, which embedding model
/// built it, and whether the service can still reach Postgres.
pub async fn status(
    State(state): State<Arc<AppState>>,
    _admin: AdminAuth,
) -> Result<Json<Value>, AppError> {
    match rag::health(&state.http, &state.config.rag_service_url).await {
        Ok(health) => Ok(Json(json!({
            "success": true,
            "reachable": true,
            "rebuild_enabled": state.config.rag_rebuild_secret.is_some(),
            "data": health,
        }))),
        // A down RAG is an expected state, not a 5xx: the panel renders it as
        // offline and keeps the rest of the dashboard usable.
        Err(e) => Ok(Json(json!({
            "success": true,
            "reachable": false,
            "rebuild_enabled": state.config.rag_rebuild_secret.is_some(),
            "message": e,
        }))),
    }
}

/// Full reindex. Needed after the chunk *text* changes (the RAG service only
/// self-rebuilds when the chunk count or embedding dimension differs), which is
/// otherwise a manual curl with a 128-char secret.
pub async fn rebuild(
    State(state): State<Arc<AppState>>,
    _admin: AdminAuth,
) -> Result<Json<Value>, AppError> {
    let Some(secret) = &state.config.rag_rebuild_secret else {
        return Err(AppError::BadRequest(
            "RAG_REBUILD_SECRET is not set on the server; rebuild is disabled".into(),
        ));
    };

    let body = rag::rebuild(&state.http, &state.config.rag_service_url, secret)
        .await
        .map_err(AppError::BadGateway)?;

    let chunks = body.get("chunks").and_then(Value::as_i64).unwrap_or(0);
    tracing::info!("[RAG] admin-triggered rebuild complete: {chunks} chunks");
    Ok(Json(json!({
        "success": true,
        "message": format!("Index rebuilt: {chunks} chunks"),
        "data": body,
    })))
}
