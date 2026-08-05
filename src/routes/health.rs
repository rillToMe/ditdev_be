use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use crate::controllers;
use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(controllers::health::get_health))
        .route("/detailed", get(controllers::health::get_detailed_health))
        .route("/ping", get(controllers::health::ping))
        .route("/database", get(controllers::health::get_database_health))
}
