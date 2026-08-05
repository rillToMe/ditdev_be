use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::error::{ProvideErrorMetadata, SdkError};
use aws_sdk_s3::Client;

use crate::config::AppConfig;
use crate::error::AppError;

/// Format an `SdkError` so the real R2 response (status/error code) is visible
/// instead of the generic "service error" Display.
fn fmt_s3_err<E: ProvideErrorMetadata + std::fmt::Debug, R: std::fmt::Debug>(
    e: &SdkError<E, R>,
) -> String {
    match e {
        SdkError::ServiceError(se) => {
            let msg = se.err().meta().message().unwrap_or("(no message)");
            format!("service error: {msg}")
        }
        SdkError::TimeoutError(_) => "timeout".to_string(),
        SdkError::DispatchFailure(f) => format!("dispatch failure: {f:?}"),
        other => format!("{other:?}"),
    }
}

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
        // R2 rejects the virtual-hosted signature from the Rust SDK ("Access Denied");
        // path-style addressing signs and resolves correctly.
        .force_path_style(true)
        // Documented Cloudflare R2 fix for aws-sdk-rust: the SDK sends a CRC-32
        // checksum header by default (WhenSupported), which R2 rejects. Only send
        // checksums when required.
        .request_checksum_calculation(aws_sdk_s3::config::RequestChecksumCalculation::WhenRequired)
        .response_checksum_validation(aws_sdk_s3::config::ResponseChecksumValidation::WhenRequired)
        .build();

    Client::from_conf(s3_config)
}

/// Normalized public base: no trailing slash, and always with a scheme.
///
/// `R2_PUBLIC_URL` may be configured bare (`cdn.example.com`). Returning that
/// as-is produces a schemeless URL that the browser resolves relative to the
/// current origin (`localhost:5173/cdn.example.com/...`), so the image 404s.
fn public_base(config: &AppConfig) -> String {
    normalize_base(&config.r2.public_url)
}

fn normalize_base(raw: &str) -> String {
    let base = raw.trim().trim_end_matches('/');
    if base.starts_with("http://") || base.starts_with("https://") {
        base.to_string()
    } else {
        format!("https://{base}")
    }
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
        .map_err(|e| AppError::Internal(format!("R2 upload failed: {}", fmt_s3_err(&e))))?;
    Ok(format!("{}/{key}", public_base(config)))
}

/// Delete an object from R2 by key.
pub async fn delete_file(client: &Client, config: &AppConfig, key: &str) -> Result<(), AppError> {
    client
        .delete_object()
        .bucket(&config.r2.bucket_name)
        .key(key)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("R2 delete failed: {}", fmt_s3_err(&e))))?;
    Ok(())
}

/// Extract the object key from a public R2 URL.
/// `https://cdn.example.com/projects/img.png` → `projects/img.png`.
/// Matches with or without scheme, so URLs stored before the base was normalized still resolve.
pub fn extract_key_from_url(config: &AppConfig, public_url: &str) -> Option<String> {
    key_from_url(&public_base(config), public_url)
}

fn key_from_url(base: &str, public_url: &str) -> Option<String> {
    let bare = base.trim_start_matches("https://").trim_start_matches("http://");
    for prefix in [format!("{base}/"), format!("http://{bare}/"), format!("{bare}/")] {
        if let Some(key) = public_url.strip_prefix(&prefix) {
            return (!key.is_empty()).then(|| key.to_string());
        }
    }
    // Fallback: the public domain may have changed since the URL was stored
    // (.site → .it.com). The object still lives under the same bucket with the
    // same `{type}/{filename}` key, so take whatever follows the host.
    let rest = public_url
        .strip_prefix("https://")
        .or_else(|| public_url.strip_prefix("http://"))?;
    let (_host, key) = rest.split_once('/')?;
    (!key.is_empty()).then(|| key.to_string())
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

#[cfg(test)]
mod tests {
    use super::{key_from_url, normalize_base};

    #[test]
    fn adds_scheme_when_missing() {
        assert_eq!(normalize_base("cdn.example.com"), "https://cdn.example.com");
        assert_eq!(normalize_base("cdn.example.com/"), "https://cdn.example.com");
        assert_eq!(normalize_base("https://cdn.example.com/"), "https://cdn.example.com");
        assert_eq!(normalize_base("http://localhost:9000"), "http://localhost:9000");
    }

    #[test]
    fn extracts_key_across_hosts_and_schemes() {
        let base = "https://cdn.new.com";
        let k = Some("projects/a.jpg".to_string());
        assert_eq!(key_from_url(base, "https://cdn.new.com/projects/a.jpg"), k);
        assert_eq!(key_from_url(base, "cdn.new.com/projects/a.jpg"), k);
        // stored under the previous custom domain — same bucket, same key
        assert_eq!(key_from_url(base, "https://cdn.old.site/projects/a.jpg"), k);
        assert_eq!(key_from_url(base, "https://cdn.new.com/"), None);
        assert_eq!(key_from_url(base, "not-a-url"), None);
    }
}
