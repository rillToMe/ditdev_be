use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::config::AppConfig;
use crate::error::AppError;

/// Connect to PostgreSQL and apply embedded migrations at startup.
pub async fn connect(config: &AppConfig) -> Result<PgPool, AppError> {
    let url = if config.db_ssl_reject_unauthorized {
        config.database_url.clone()
    } else {
        append_sslmode(&config.database_url, "require")
    };

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .map_err(|e| AppError::Internal(format!("failed to connect to database: {e}")))?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|e| AppError::Internal(format!("migration failed: {e}")))?;

    Ok(pool)
}

/// `sslmode=require` disables certificate verification (equivalent of the Node
/// `rejectUnauthorized: false`), used only when the operator opts out via env.
fn append_sslmode(url: &str, mode: &str) -> String {
    if url.contains("sslmode=") {
        url.to_string()
    } else if url.contains('?') {
        format!("{url}&sslmode={mode}")
    } else {
        format!("{url}?sslmode={mode}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sslmode_appended_correctly() {
        assert_eq!(
            append_sslmode("postgresql://u:p@host/db", "require"),
            "postgresql://u:p@host/db?sslmode=require"
        );
        assert_eq!(
            append_sslmode("postgresql://u:p@host/db?x=1", "require"),
            "postgresql://u:p@host/db?x=1&sslmode=require"
        );
        // never duplicate an existing sslmode
        assert_eq!(
            append_sslmode("postgresql://u:p@host/db?sslmode=disable", "require"),
            "postgresql://u:p@host/db?sslmode=disable"
        );
    }
}
