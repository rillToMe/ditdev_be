use std::sync::Arc;

use axum::routing::post;
use axum::Router;

use crate::controllers;
use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/", post(controllers::chat::send_message))
}
