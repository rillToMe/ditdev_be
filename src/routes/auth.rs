use std::sync::Arc;

use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use axum::Router;

use crate::controllers;
use crate::middleware::rate;
use crate::state::AppState;

pub fn router(state: &Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/login",
            post(controllers::auth::login).layer(from_fn_with_state(state.clone(), rate::auth_limit)),
        )
        .route(
            "/register",
            post(controllers::auth::register).layer(from_fn_with_state(state.clone(), rate::auth_limit)),
        )
        .route("/verify", get(controllers::auth::verify))
        .route("/logout", post(controllers::auth::logout))
        .route("/sessions", get(controllers::auth::sessions))
}
