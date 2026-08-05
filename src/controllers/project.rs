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
struct ProjectRow {
    id: i32,
    title: String,
    description: String,
    thumbnail: Option<String>,
    tags: Option<Vec<String>>,
    created_at: PgTimestamp,
    updated_at: PgTimestamp,
    links: Option<Value>,
}

#[derive(Deserialize)]
struct ProjectLinkInput {
    #[serde(rename = "type")]
    type_: String,
    url: String,
}

#[derive(Deserialize)]
pub(crate) struct ProjectRequest {
    title: Option<String>,
    description: Option<String>,
    thumbnail: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    links: Option<Vec<ProjectLinkInput>>,
}

#[derive(FromRow)]
struct ProjectIdRow {
    id: i32,
}

const PROJECT_SELECT: &str = r#"
    SELECT p.*,
      json_agg(json_build_object('type', pl.type, 'url', pl.url)) FILTER (WHERE pl.id IS NOT NULL) as links
    FROM projects p
    LEFT JOIN project_links pl ON p.id = pl.project_id
"#;

pub async fn get_all(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let rows = sqlx::query_as::<_, ProjectRow>(&format!(
        "{PROJECT_SELECT} GROUP BY p.id ORDER BY p.created_at DESC"
    ))
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
    match fetch_project(&state.db, id).await? {
        Some(row) => Ok(Json(json!({ "success": true, "data": serde_json::to_value(&row).unwrap() }))
            .into_response()),
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "message": "Project not found" })),
        )
            .into_response()),
    }
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    _admin: AdminAuth,
    Json(body): Json<ProjectRequest>,
) -> Result<Response, AppError> {
    let missing = body
        .title
        .as_deref()
        .map(|t| t.is_empty())
        .unwrap_or(true)
        || body
            .description
            .as_deref()
            .map(|d| d.is_empty())
            .unwrap_or(true);
    if missing {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "Title and description are required" })),
        )
            .into_response());
    }

    let mut tx = state.db.begin().await?;
    let inserted = sqlx::query_as::<_, ProjectIdRow>(
        "INSERT INTO projects (title, description, thumbnail, tags) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(body.title)
    .bind(body.description)
    .bind(body.thumbnail.as_deref())
    .bind(&body.tags)
    .fetch_one(&mut *tx)
    .await?;

    insert_links(&mut tx, inserted.id, &body.links).await?;
    tx.commit().await?;

    let complete = fetch_project(&state.db, inserted.id)
        .await?
        .ok_or_else(|| AppError::Internal("created project not found".into()))?;
    let value = serde_json::to_value(&complete)?;
    state.rag.on_project_created(&value);
    Ok((
        StatusCode::CREATED,
        Json(json!({ "success": true, "message": "Project created successfully", "data": value })),
    )
        .into_response())
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    _admin: AdminAuth,
    Path(id): Path<i32>,
    Json(body): Json<ProjectRequest>,
) -> Result<Response, AppError> {
    let mut tx = state.db.begin().await?;
    let old: Option<(Option<String>,)> =
        sqlx::query_as("SELECT thumbnail FROM projects WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((old_thumbnail,)) = old else {
        tx.rollback().await?;
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "message": "Project not found" })),
        )
            .into_response());
    };

    sqlx::query(
        "UPDATE projects SET title=$1, description=$2, thumbnail=$3, tags=$4, updated_at=CURRENT_TIMESTAMP WHERE id=$5",
    )
    .bind(body.title)
    .bind(body.description)
    .bind(body.thumbnail.as_deref())
    .bind(&body.tags)
    .bind(id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM project_links WHERE project_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    insert_links(&mut tx, id, &body.links).await?;
    tx.commit().await?;

    // Delete old thumbnail from R2 only if it changed (Node parity).
    if let Some(old) = &old_thumbnail {
        if Some(old.clone()) != body.thumbnail {
            r2::delete_from_r2(&state.r2, &state.config, Some(old)).await;
        }
    }

    let complete = fetch_project(&state.db, id)
        .await?
        .ok_or_else(|| AppError::Internal("updated project not found".into()))?;
    let value = serde_json::to_value(&complete)?;
    state.rag.on_project_updated(&value);
    Ok(Json(json!({ "success": true, "message": "Project updated successfully", "data": value }))
        .into_response())
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    _admin: AdminAuth,
    Path(id): Path<i32>,
) -> Result<Response, AppError> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT thumbnail FROM projects WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let Some((thumbnail,)) = row else {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "message": "Project not found" })),
        )
            .into_response());
    };

    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    r2::delete_from_r2(&state.r2, &state.config, thumbnail.as_deref()).await;
    state.rag.on_project_deleted(id);
    Ok(Json(json!({ "success": true, "message": "Project deleted successfully" })).into_response())
}

async fn insert_links(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    project_id: i32,
    links: &Option<Vec<ProjectLinkInput>>,
) -> Result<(), AppError> {
    if let Some(links) = links {
        for link in links {
            sqlx::query("INSERT INTO project_links (project_id, type, url) VALUES ($1, $2, $3)")
                .bind(project_id)
                .bind(&link.type_)
                .bind(&link.url)
                .execute(&mut **tx)
                .await?;
        }
    }
    Ok(())
}

async fn fetch_project(
    pool: &sqlx::PgPool,
    id: i32,
) -> Result<Option<ProjectRow>, AppError> {
    Ok(sqlx::query_as::<_, ProjectRow>(&format!(
        "{PROJECT_SELECT} WHERE p.id = $1 GROUP BY p.id"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?)
}
