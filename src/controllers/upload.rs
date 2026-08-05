use std::path::Path as FsPath;
use std::sync::Arc;

use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rand::Rng;
use serde::Deserialize;
use serde_json::json;

use crate::error::AppError;
use crate::middleware::auth::AdminAuth;
use crate::services::r2;
use crate::state::AppState;

const IMAGE_TYPES: [&str; 5] = ["jpeg", "jpg", "png", "gif", "webp"];
const IMAGE_MAX: usize = 5 * 1024 * 1024;
const PDF_MAX: usize = 10 * 1024 * 1024;

struct FileUpload {
    file_name: String,
    content_type: String,
    bytes: Vec<u8>,
    size: usize,
}

#[derive(Deserialize)]
pub(crate) struct DeleteQuery {
    #[serde(rename = "type")]
    type_: Option<String>,
}

pub async fn upload_image(
    State(state): State<Arc<AppState>>,
    _admin: AdminAuth,
    mut multipart: Multipart,
) -> Result<Response, AppError> {
    let mut type_field = "projects".to_string();
    let mut upload: Option<FileUpload> = None;

    while let Some(field) = multipart.next_field().await? {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "image" => {
                let file_name = field.file_name().unwrap_or("").to_string();
                let content_type = field.content_type().unwrap_or("").to_string();
                let bytes = field.bytes().await?.to_vec();
                let size = bytes.len();
                upload = Some(FileUpload { file_name, content_type, bytes, size });
            }
            "type" => {
                type_field = field.text().await.unwrap_or_default();
            }
            _ => {}
        }
    }

    let Some(file) = upload else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "No file uploaded" })),
        )
            .into_response());
    };

    let ext = lowercase_ext(&file.file_name);
    if !contains_any(&file.content_type, &IMAGE_TYPES) || !contains_any(&ext, &IMAGE_TYPES) {
        return Err(AppError::BadRequest(
            "Only image files are allowed (jpeg, jpg, png, gif, webp)".into(),
        ));
    }
    if file.size > IMAGE_MAX {
        return Err(AppError::BadRequest("File too large (max 5 MB)".into()));
    }

    let filename = sanitize_filename(&file.file_name);
    let key = format!("{type_field}/{filename}");
    let public_url = r2::upload_file(&state.r2, &state.config, &key, file.bytes, &file.content_type).await?;

    Ok(Json(json!({
        "success": true,
        "message": "File uploaded successfully",
        "data": { "filename": filename, "path": public_url, "size": file.size, "type": type_field },
    }))
    .into_response())
}

pub async fn upload_pdf(
    State(state): State<Arc<AppState>>,
    _admin: AdminAuth,
    mut multipart: Multipart,
) -> Result<Response, AppError> {
    let mut upload: Option<FileUpload> = None;

    while let Some(field) = multipart.next_field().await? {
        if field.name().unwrap_or("").to_string() == "pdf" {
            let file_name = field.file_name().unwrap_or("").to_string();
            let content_type = field.content_type().unwrap_or("").to_string();
            let bytes = field.bytes().await?.to_vec();
            let size = bytes.len();
            upload = Some(FileUpload { file_name, content_type, bytes, size });
        }
    }

    let Some(file) = upload else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "No PDF file uploaded" })),
        )
            .into_response());
    };

    let ext = lowercase_ext(&file.file_name);
    if !file.content_type.contains("pdf") || !ext.contains("pdf") {
        return Err(AppError::BadRequest("Only PDF files are allowed".into()));
    }
    if file.size > PDF_MAX {
        return Err(AppError::BadRequest("File too large (max 10 MB)".into()));
    }

    let filename = sanitize_filename(&file.file_name);
    let key = format!("pdf_certif/{filename}");
    let public_url = r2::upload_file(&state.r2, &state.config, &key, file.bytes, "application/pdf").await?;

    Ok(Json(json!({
        "success": true,
        "message": "PDF uploaded successfully",
        "data": { "filename": filename, "path": public_url, "size": file.size, "type": "pdf_certif" },
    }))
    .into_response())
}

pub async fn delete_image(
    State(state): State<Arc<AppState>>,
    _admin: AdminAuth,
    Path(filename): Path<String>,
    Query(params): Query<DeleteQuery>,
) -> Result<Response, AppError> {
    let type_field = params.type_.as_deref().unwrap_or("projects");
    let key = format!("{type_field}/{filename}");
    r2::delete_file(&state.r2, &state.config, &key).await?;
    Ok(Json(json!({ "success": true, "message": "File deleted successfully" })).into_response())
}

fn lowercase_ext(file_name: &str) -> String {
    FsPath::new(file_name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// Port of `sanitizeFilename` in uploadController.js — same replacement rules,
/// collapse, trim, and `{Date.now()}-{rand}` suffix.
pub fn sanitize_filename(original: &str) -> String {
    let path = FsPath::new(original);
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| original.to_string());

    let name = sanitize_stem(&stem);
    let suffix = format!(
        "{}-{}",
        chrono::Utc::now().timestamp_millis(),
        rand::thread_rng().gen_range(0..1_000_000_000)
    );
    format!("{name}-{suffix}{ext}")
}

fn sanitize_stem(stem: &str) -> String {
    // whitespace → _, special chars → -, disallowed chars removed (steps 1–3)
    let mut chars: Vec<char> = Vec::with_capacity(stem.len());
    let mut prev_ws = false;
    for c in stem.chars() {
        if c.is_whitespace() {
            if !prev_ws {
                chars.push('_');
            }
            prev_ws = true;
        } else {
            prev_ws = false;
            match c {
                '#' | '&' | '%' | '?' | '=' | '+' | '@' | '!' | '$' | '(' | ')' | '[' | ']'
                | '{' | '}' | '<' | '>' => chars.push('-'),
                '\'' | '"' | '`' | ';' | ':' | '|' | '\\' | '/' => {}
                other => chars.push(other),
            }
        }
    }

    // collapse - and _ runs (steps 4–5)
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    for c in chars {
        if let Some(&last) = out.last() {
            if (c == '-' || c == '_') && c == last {
                continue;
            }
        }
        out.push(c);
    }

    // trim leading/trailing -_ (step 6); default "file" if empty (step 7)
    let trimmed: String = out.into_iter().collect();
    let trimmed = trimmed.trim_matches(|c| c == '-' || c == '_').to_string();
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_whitespace_and_specials() {
        let s = sanitize_filename("My Photo #1!.png");
        assert!(s.starts_with("My_Photo_-1-"), "got: {s}");
        assert!(s.ends_with(".png"));
    }

    #[test]
    fn removes_dangerous_chars() {
        // Note: `\` and `/` are path separators, stripped by basename — use the
        // removal-set chars that survive the path split (`'`, `;`, `|`, `:`).
        let s = sanitize_filename("a'b;c|d:e.png");
        assert!(s.starts_with("abcde-"), "got: {s}");
    }

    #[test]
    fn defaults_to_file_when_emptied() {
        let s = sanitize_filename("###.png");
        assert!(s.starts_with("file-"), "got: {s}");
    }

    #[test]
    fn collapses_runs() {
        let s = sanitize_filename("a__b--c .png");
        assert!(s.starts_with("a_b-c-"), "got: {s}");
    }
}
