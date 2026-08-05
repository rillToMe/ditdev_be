use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::FromRow;

use crate::error::AppError;
use crate::middleware::auth::{sign_jwt, AdminAuth};
use crate::state::AppState;
use crate::util::PgTimestamp;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(FromRow)]
struct AdminRow {
    id: i32,
    username: String,
    password: String,
}

#[derive(FromRow)]
struct RegisterRow {
    id: i32,
    username: String,
    created_at: PgTimestamp,
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginRequest>,
) -> Result<Response, AppError> {
    let (Some(username), Some(password)) = (body.username, body.password) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "Please provide username and password" })),
        )
            .into_response());
    };

    let admin = sqlx::query_as::<_, AdminRow>("SELECT id, username, password FROM admins WHERE username = $1")
        .bind(&username)
        .fetch_optional(&state.db)
        .await?;
    let Some(admin) = admin else {
        return Ok((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "success": false, "message": "Invalid credentials" })),
        )
            .into_response());
    };

    let stored = admin.password.clone();
    let pass = password.clone();
    let is_match = tokio::task::spawn_blocking(move || bcrypt::verify(&pass, &stored))
        .await
        .map_err(|e| AppError::Internal(format!("bcrypt task error: {e}")))?
        .map_err(|e| AppError::Internal(format!("bcrypt verify error: {e}")))?;
    if !is_match {
        return Ok((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "success": false, "message": "Invalid credentials" })),
        )
            .into_response());
    }

    let token = sign_jwt(&state.config.jwt_secret, admin.id, &admin.username, &state.config.jwt_expire)?;
    tracing::info!("Admin logged in: {username}");
    Ok(Json(json!({
        "success": true,
        "token": token,
        "admin": { "id": admin.id, "username": admin.username },
        "expiresIn": state.config.jwt_expire,
    }))
    .into_response())
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    admin: AdminAuth,
    Json(body): Json<RegisterRequest>,
) -> Result<Response, AppError> {
    let (Some(username), Some(password)) = (body.username, body.password) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "Please provide username and password" })),
        )
            .into_response());
    };
    if password.len() < 8 {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "Password must be at least 8 characters long" })),
        )
            .into_response());
    }

    let exists = sqlx::query("SELECT id FROM admins WHERE username = $1")
        .bind(&username)
        .fetch_optional(&state.db)
        .await?;
    if exists.is_some() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "Admin already exists" })),
        )
            .into_response());
    }

    let pass = password.clone();
    let hashed = tokio::task::spawn_blocking(move || bcrypt::hash(&pass, 12))
        .await
        .map_err(|e| AppError::Internal(format!("bcrypt task error: {e}")))?
        .map_err(|e| AppError::Internal(format!("bcrypt hash error: {e}")))?;

    let row = sqlx::query_as::<_, RegisterRow>(
        "INSERT INTO admins (username, password) VALUES ($1, $2) RETURNING id, username, created_at",
    )
    .bind(&username)
    .bind(&hashed)
    .fetch_one(&state.db)
    .await?;

    tracing::info!("New admin created: {username} by {}", admin.claims.username);
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "success": true,
            "message": "Admin created successfully",
            "admin": { "id": row.id, "username": row.username, "created_at": row.created_at.to_rfc3339() },
        })),
    )
        .into_response())
}

pub async fn verify(admin: AdminAuth) -> Json<Value> {
    Json(json!({
        "success": true,
        "admin": serde_json::to_value(&admin.claims).unwrap_or(Value::Null),
    }))
}

pub async fn logout(
    State(state): State<Arc<AppState>>,
    admin: AdminAuth,
) -> Result<Json<Value>, AppError> {
    state.auth.blacklist(&admin.token);
    tracing::info!("Admin logged out: {}", admin.claims.username);
    Ok(Json(json!({ "success": true, "message": "Logged out successfully" })))
}

pub async fn sessions(State(state): State<Arc<AppState>>, _admin: AdminAuth) -> Json<Value> {
    let sessions = state.auth.active_sessions();
    Json(json!({ "success": true, "count": sessions.len(), "sessions": sessions }))
}
