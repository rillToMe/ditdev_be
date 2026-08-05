use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use crate::controllers;
use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/",
            get(controllers::stats::get_all).post(controllers::stats::create),
        )
        .route(
            "/{key}",
            get(controllers::stats::get_one)
                .put(controllers::stats::update)
                .delete(controllers::stats::delete),
        )
}
