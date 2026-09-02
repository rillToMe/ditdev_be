use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use crate::controllers;
use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status", get(controllers::rag::status))
        .route("/rebuild", post(controllers::rag::rebuild))
}
