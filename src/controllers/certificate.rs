use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::FromRow;

use crate::error::AppError;
use crate::middleware::auth::AdminAuth;
use crate::services::r2;
use crate::state::AppState;
use crate::util::PgTimestamp;

#[derive(FromRow, Serialize)]
struct CertificateRow {
    id: i32,
    title: String,
    provider: String,
    thumbnail: Option<String>,
    issue_date: Option<String>,
    credential_url: Option<String>,
    pdf_file: String,
    created_at: PgTimestamp,
}

#[derive(Deserialize)]
pub(crate) struct CertificateRequest {
    title: Option<String>,
    provider: Option<String>,
    thumbnail: Option<String>,
    issue_date: Option<String>,
    credential_url: Option<String>,
    pdf_file: Option<String>,
}

pub async fn get_all(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let rows =
        sqlx::query_as::<_, CertificateRow>("SELECT * FROM certificates ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await?;
    let data: Vec<Value> = rows
        .iter()
        .map(|r| serde_json::to_value(r).unwrap())
        .collect();
    Ok(Json(json!({ "success": true, "count": data.len(), "data": data })))
}

pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> Result<Response, AppError> {
    let row = sqlx::query_as::<_, CertificateRow>("SELECT * FROM certificates WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    match row {
        Some(row) => Ok(Json(json!({ "success": true, "data": serde_json::to_value(&row).unwrap() }))
            .into_response()),
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "message": "Certificate not found" })),
        )
            .into_response()),
    }
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    _admin: AdminAuth,
    Json(body): Json<CertificateRequest>,
) -> Result<Response, AppError> {
    let (Some(title), Some(provider), Some(pdf_file)) = (body.title, body.provider, body.pdf_file)
    else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "Title, provider, and PDF file are required" })),
        )
            .into_response());
    };

    let row = sqlx::query_as::<_, CertificateRow>(
        "INSERT INTO certificates (title, provider, thumbnail, issue_date, credential_url, pdf_file) VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
    )
    .bind(&title)
    .bind(&provider)
    .bind(body.thumbnail.as_deref())
    .bind(body.issue_date)
    .bind(body.credential_url.as_deref())
    .bind(&pdf_file)
    .fetch_one(&state.db)
    .await?;

    let value = serde_json::to_value(&row)?;
    state.rag.on_certificate_created(&value);
    Ok((
        StatusCode::CREATED,
        Json(json!({ "success": true, "message": "Certificate created successfully", "data": value })),
    )
        .into_response())
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    _admin: AdminAuth,
    Path(id): Path<i32>,
    Json(body): Json<CertificateRequest>,
) -> Result<Response, AppError> {
    let (Some(title), Some(provider)) = (body.title, body.provider) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "Title and provider are required" })),
        )
            .into_response());
    };

    let old: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT thumbnail, pdf_file FROM certificates WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let Some((old_thumb, old_pdf)) = old else {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "message": "Certificate not found" })),
        )
            .into_response());
    };

    let row = sqlx::query_as::<_, CertificateRow>(
        "UPDATE certificates SET title=$1, provider=$2, thumbnail=$3, issue_date=$4, credential_url=$5, pdf_file=$6 WHERE id=$7 RETURNING *",
    )
    .bind(&title)
    .bind(&provider)
    .bind(body.thumbnail.as_deref())
    .bind(body.issue_date)
    .bind(body.credential_url.as_deref())
    .bind(body.pdf_file.as_deref())
    .bind(id)
    .fetch_one(&state.db)
    .await?;

    if let Some(old) = &old_thumb {
        if Some(old.clone()) != body.thumbnail {
            r2::delete_from_r2(&state.r2, &state.config, Some(old)).await;
        }
    }
    if let Some(old) = &old_pdf {
        if Some(old.clone()) != body.pdf_file {
            r2::delete_from_r2(&state.r2, &state.config, Some(old)).await;
        }
    }

    let value = serde_json::to_value(&row)?;
    state.rag.on_certificate_updated(&value);
    Ok(Json(json!({ "success": true, "message": "Certificate updated successfully", "data": value }))
        .into_response())
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    _admin: AdminAuth,
    Path(id): Path<i32>,
) -> Result<Response, AppError> {
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT thumbnail, pdf_file FROM certificates WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let Some((thumb, pdf)) = row else {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "message": "Certificate not found" })),
        )
            .into_response());
    };

    sqlx::query("DELETE FROM certificates WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    state.rag.on_certificate_deleted(id);
    r2::delete_from_r2(&state.r2, &state.config, thumb.as_deref()).await;
    r2::delete_from_r2(&state.r2, &state.config, pdf.as_deref()).await;
    Ok(Json(json!({ "success": true, "message": "Certificate deleted successfully" })).into_response())
}

