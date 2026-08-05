use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use crate::controllers;
use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(controllers::xp::get_xp))
        .route("/tick", post(controllers::xp::tick_xp))
}
