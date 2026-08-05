use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::Client;

use crate::config::AppConfig;
use crate::error::AppError;

/// Build an S3 client pointed at the Cloudflare R2 endpoint (S3-compatible),
/// mirroring `config/cloudflareR2.js`.
pub fn client(config: &AppConfig) -> Client {
    let creds = Credentials::new(
        &config.r2.access_key_id,
        &config.r2.secret_access_key,
        None,
        None,
        "r2",
    );
    let endpoint = format!("https://{}.r2.cloudflarestorage.com", config.r2.account_id);

    let s3_config = aws_sdk_s3::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("auto"))
        .credentials_provider(creds)
        .endpoint_url(endpoint)
        .build();

    Client::from_conf(s3_config)
}

/// Upload bytes to R2 under `key`, return the public URL (`{R2_PUBLIC_URL}/{key}`).
pub async fn upload_file(
    client: &Client,
    config: &AppConfig,
    key: &str,
    bytes: Vec<u8>,
    content_type: &str,
) -> Result<String, AppError> {
    client
        .put_object()
        .bucket(&config.r2.bucket_name)
        .key(key)
        .body(bytes.into())
        .content_type(content_type)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("R2 upload failed: {e}")))?;
    Ok(format!("{}/{key}", config.r2.public_url.trim_end_matches('/')))
}

/// Delete an object from R2 by key.
pub async fn delete_file(client: &Client, config: &AppConfig, key: &str) -> Result<(), AppError> {
    client
        .delete_object()
        .bucket(&config.r2.bucket_name)
        .key(key)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("R2 delete failed: {e}")))?;
    Ok(())
}

/// Extract the object key from a public R2 URL.
/// `https://cdn.example.com/projects/img.png` → `projects/img.png`; None if not under the base.
pub fn extract_key_from_url(config: &AppConfig, public_url: &str) -> Option<String> {
    let base = format!("{}/", config.r2.public_url.trim_end_matches('/'));
    public_url.strip_prefix(&base).map(|k| k.to_string())
}

/// Shared "delete object referenced by a public URL, swallowing errors" helper.
///
/// Hardening H1: the Node app duplicated this identically in projectController and
/// certificateController; this is the single implementation both controllers use.
pub async fn delete_from_r2(client: &Client, config: &AppConfig, public_url: Option<&str>) {
    let Some(url) = public_url else { return };
    match extract_key_from_url(config, url) {
        Some(key) => {
            tracing::info!("Deleting from R2: {key}");
            if let Err(e) = delete_file(client, config, &key).await {
                tracing::error!("Error deleting from R2: {e}");
            }
        }
        None => tracing::warn!("URL not recognized, skip deletion: {url}"),
    }
}
