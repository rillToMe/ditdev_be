use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use aws_sdk_s3::Client as S3Client;
use sqlx::PgPool;

use crate::config::AppConfig;
use crate::middleware::auth::AuthStore;
use crate::middleware::rate::RateLimiters;
use crate::services::github::GitHubCache;
use crate::services::rag::RagService;

/// Shared application state, wrapped in `Arc` and passed via axum `State`.
pub struct AppState {
    pub db: PgPool,
    pub config: AppConfig,
    pub r2: S3Client,
    pub started: Instant,
    pub auth: AuthStore,
    pub rag: RagService,
    pub limiters: RateLimiters,
    pub http: reqwest::Client,
    pub github_cache: GitHubCache,
    /// per-IP last tick timestamp (ms) for the XP cooldown — ponytail: in-memory, single instance
    pub xp_ticks: Mutex<HashMap<String, i64>>,
}

impl AppState {
    pub fn new(
        db: PgPool,
        config: AppConfig,
        r2: S3Client,
        auth: AuthStore,
        rag: RagService,
        limiters: RateLimiters,
        http: reqwest::Client,
        github_cache: GitHubCache,
    ) -> Self {
        Self {
            db,
            config,
            r2,
            started: Instant::now(),
            auth,
            rag,
            limiters,
            http,
            github_cache,
            xp_ticks: Mutex::new(HashMap::new()),
        }
    }
}
