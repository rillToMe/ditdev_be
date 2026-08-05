use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use crate::controllers;
use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/activity", get(controllers::github::get_activity))
        .route("/heatmap", get(controllers::github::get_heatmap))
}
