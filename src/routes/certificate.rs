use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use crate::controllers;
use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/",
            get(controllers::certificate::get_all).post(controllers::certificate::create),
        )
        .route(
            "/{id}",
            get(controllers::certificate::get_one)
                .put(controllers::certificate::update)
                .delete(controllers::certificate::delete),
        )
}
