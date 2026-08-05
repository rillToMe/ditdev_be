use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::middleware::from_fn_with_state;
use axum::routing::{delete, post};
use axum::Router;

use crate::controllers;
use crate::middleware::rate;
use crate::state::AppState;

/// 11 MB body cap: covers the 10 MB PDF limit + multipart overhead. Per-file size
/// limits are enforced in the handlers (5 MB image / 10 MB PDF).
const MAX_BODY: usize = 11 * 1024 * 1024;

pub fn router(state: &Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/", post(controllers::upload::upload_image))
        .route("/pdf", post(controllers::upload::upload_pdf))
        .route("/{filename}", delete(controllers::upload::delete_image))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(from_fn_with_state(state.clone(), rate::upload_limit))
}
