pub mod auth;
pub mod certificate;
pub mod chat;
pub mod contact;
pub mod github;
pub mod health;
pub mod project;
pub mod rag;
pub mod stats;
pub mod upload;
pub mod xp;

use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

pub async fn root_info() -> Json<Value> {
    Json(json!({
        "message": "Portfolio API",
        "version": "2.0.0",
        "endpoints": {
            "health": "/api/health",
            "healthDetailed": "/api/health/detailed",
            "ping": "/api/health/ping",
            "auth": "/api/auth",
            "projects": "/api/projects",
            "certificates": "/api/certificates",
            "stats": "/api/stats",
        }
    }))
}

pub async fn not_found() -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "success": false, "message": "Route not found" })),
    )
}
