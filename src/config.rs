use crate::error::AppError;

/// All runtime configuration, loaded from the environment (or `.env`).
/// New env vars get added here as phases land; everything is documented in `.env.example`.
#[derive(Clone, Debug)]
pub struct AppConfig {
    pub port: u16,
    pub app_env: String,
    pub log_level: String,
    pub database_url: String,
    pub client_url: Option<String>,
    pub admin_url: Option<String>,
    pub db_ssl_reject_unauthorized: bool,
    pub jwt_secret: String,
    pub jwt_expire: String,
    /// Unused since chat moved to Xkiro; kept for the cerebras fallback service.
    #[allow(dead_code)]
    pub cerebras_api_key: Option<String>,
    #[allow(dead_code)]
    pub cerebras_model: String,
    pub xkiro_api_key: Option<String>,
    pub xkiro_model: String,
    pub discord_webhook_url: Option<String>,
    pub rag_service_url: String,
    /// Sent as `X-RAG-Secret` on mutating RAG calls. Only needed when the RAG
    /// service is not on localhost; it rejects unsigned writes once it has one.
    pub rag_api_secret: Option<String>,
    /// Body secret for the RAG service's `/rebuild`. Separate from `rag_api_secret`
    /// on purpose: a full rebuild drops the collection before re-embedding, so it
    /// is gated by its own credential. Absent = the admin rebuild button is
    /// disabled rather than silently failing.
    pub rag_rebuild_secret: Option<String>,
    pub github_username: String,
    pub github_token: Option<String>,
    pub r2: R2Config,
}

#[derive(Clone, Debug)]
pub struct R2Config {
    pub account_id: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket_name: String,
    pub public_url: String,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, AppError> {
        fn required(key: &str) -> Result<String, AppError> {
            std::env::var(key).map_err(|_| AppError::Config(format!("missing required env var: {key}")))
        }

        let r2 = R2Config {
            account_id: required("R2_ACCOUNT_ID")?,
            access_key_id: required("R2_ACCESS_KEY_ID")?,
            secret_access_key: required("R2_SECRET_ACCESS_KEY")?,
            bucket_name: required("R2_BUCKET_NAME")?,
            public_url: required("R2_PUBLIC_URL")?,
        };

        Ok(Self {
            port: env_or("PORT", "2817")
                .parse()
                .map_err(|e| AppError::Config(format!("PORT invalid: {e}")))?,
            app_env: env_or("APP_ENV", "development"),
            log_level: env_or("LOG_LEVEL", "info"),
            database_url: required("DATABASE_URL")?,
            client_url: std::env::var("CLIENT_URL").ok(),
            admin_url: std::env::var("ADMIN_URL").ok(),
            db_ssl_reject_unauthorized: env_or("DB_SSL_REJECT_UNAUTHORIZED", "true")
                .eq_ignore_ascii_case("true")
                || env_or("DB_SSL_REJECT_UNAUTHORIZED", "true") == "1",
            jwt_secret: required("JWT_SECRET")?,
            jwt_expire: env_or("JWT_EXPIRE", "24h"),
            cerebras_api_key: std::env::var("CEREBRAS_API_KEY").ok(),
            cerebras_model: env_or("CEREBRAS_MODEL", "gpt-5.5"),
            xkiro_api_key: std::env::var("XKIRO_API_KEY").ok(),
            xkiro_model: env_or("XKIRO_MODEL", "deepseek/deepseek-v4-flash"),
            discord_webhook_url: std::env::var("DISCORD_WEBHOOK_URL").ok(),
            rag_service_url: env_or("RAG_SERVICE_URL", "http://localhost:8765"),
            rag_api_secret: std::env::var("RAG_API_SECRET").ok().filter(|s| !s.is_empty()),
            rag_rebuild_secret: std::env::var("RAG_REBUILD_SECRET").ok().filter(|s| !s.is_empty()),
            github_username: env_or("GITHUB_USERNAME", "rillToMe"),
            github_token: std::env::var("GITHUB_TOKEN").ok(),
            r2,
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
