mod config;
mod controllers;
mod error;
mod middleware;
mod routes;
mod services;
mod state;
mod util;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::{header, HeaderValue, Method};
use axum::middleware::from_fn_with_state;
use axum::routing::get;
use axum::Router;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::config::AppConfig;
use crate::middleware::auth::AuthStore;
use crate::middleware::rate::RateLimiters;
use crate::services::github::GitHubCache;
use crate::services::rag::RagService;
use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let config = AppConfig::from_env()?;
    init_tracing(&config);

    let db = services::db::connect(&config).await?;
    tracing::info!("database connected, migrations applied");

    let r2 = services::r2::client(&config);
    tracing::info!("r2 client initialized");

    let auth = AuthStore::new();
    let rag = RagService::new(&config);
    let limiters = RateLimiters::new();
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let github_cache = GitHubCache::new();
    let state = Arc::new(AppState::new(db, config.clone(), r2, auth, rag, limiters, http, github_cache));

    let api = Router::new()
        .nest("/health", routes::health::router())
        .nest("/auth", routes::auth::router(&state))
        .nest("/projects", routes::project::router())
        .nest("/certificates", routes::certificate::router())
        .nest("/stats", routes::stats::router())
        .nest("/upload", routes::upload::router(&state))
        .nest("/chat", routes::chat::router())
        .nest("/contact", routes::contact::router())
        .nest("/github", routes::github::router())
        .nest("/xp", routes::xp::router())
        .layer(from_fn_with_state(state.clone(), middleware::rate::api_limit));

    let app = Router::new()
        .route("/", get(routes::root_info))
        .nest("/api", api)
        .fallback(routes::not_found)
        .layer(cors_layer(&config))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let port = config.port;
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("Server started on port {port}");
    tracing::info!("API available at: http://localhost:{port}/api");

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn init_tracing(config: &AppConfig) {
    let default_level = if config.app_env == "production" {
        "warn"
    } else {
        config.log_level.as_str()
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// CORS whitelist — mirrors server.js: exact origins, credentials, and methods.
fn cors_layer(config: &AppConfig) -> CorsLayer {
    let mut origins: Vec<&str> = vec![
        "http://localhost:5173",
        "http://localhost:5000",
        "ditdev.kyuzenstudio.com",
        "http://ditdev.kyuzenstudio.com",
    ];
    if let Some(u) = &config.client_url {
        origins.push(u.as_str());
    }
    if let Some(u) = &config.admin_url {
        origins.push(u.as_str());
    }

    let list: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(list))
        .allow_credentials(true)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received, shutting down gracefully");
}
