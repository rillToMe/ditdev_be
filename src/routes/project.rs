use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use crate::controllers;
use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/",
            get(controllers::project::get_all).post(controllers::project::create),
        )
        .route(
            "/{id}",
            get(controllers::project::get_one)
                .put(controllers::project::update)
                .delete(controllers::project::delete),
        )
}
