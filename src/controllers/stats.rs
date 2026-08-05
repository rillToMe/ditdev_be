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
use crate::state::AppState;
use crate::util::{calculate_months_diff, parse_date, PgTimestamp};

#[derive(FromRow, Serialize)]
struct StatRow {
    id: i32,
    key: String,
    value: Option<i32>,
    label: String,
    start_date: Option<chrono::NaiveDate>,
    created_at: PgTimestamp,
    updated_at: PgTimestamp,
}

#[derive(Deserialize)]
pub(crate) struct CreateStatRequest {
    key: Option<String>,
    label: Option<String>,
    value: Option<i32>,
    start_date: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct UpdateStatRequest {
    value: Option<i32>,
    label: Option<String>,
    start_date: Option<String>,
}

pub async fn get_all(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let rows = sqlx::query_as::<_, StatRow>("SELECT * FROM stats ORDER BY id ASC")
        .fetch_all(&state.db)
        .await?;
    let project_count: i32 = sqlx::query_scalar("SELECT COUNT(*)::int FROM projects")
        .fetch_one(&state.db)
        .await?;
    let data: Vec<Value> = rows.iter().map(|s| stat_json(s, project_count)).collect();
    Ok(Json(json!({ "success": true, "data": data })))
}

pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> Result<Response, AppError> {
    let row = sqlx::query_as::<_, StatRow>("SELECT * FROM stats WHERE key = $1")
        .bind(&key)
        .fetch_optional(&state.db)
        .await?;
    let Some(stat) = row else {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "message": "Stat not found" })),
        )
            .into_response());
    };
    let project_count: i32 = if stat.key == "total_projects" {
        sqlx::query_scalar("SELECT COUNT(*)::int FROM projects")
            .fetch_one(&state.db)
            .await?
    } else {
        0
    };
    Ok(Json(json!({ "success": true, "data": stat_json(&stat, project_count) })).into_response())
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    _admin: AdminAuth,
    Json(body): Json<CreateStatRequest>,
) -> Result<Response, AppError> {
    let (Some(key), Some(label)) = (body.key, body.label) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "Key and label are required" })),
        )
            .into_response());
    };
    let has_start = body
        .start_date
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if !has_start && body.value.is_none() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "Either value or start_date is required" })),
        )
            .into_response());
    }

    let result = if has_start {
        let start_date = parse_date(body.start_date)?;
        sqlx::query_as::<_, StatRow>(
            "INSERT INTO stats (key, label, start_date) VALUES ($1, $2, $3) RETURNING *",
        )
        .bind(&key)
        .bind(&label)
        .bind(start_date)
        .fetch_one(&state.db)
        .await
        .map_err(unique_violation)?
    } else {
        sqlx::query_as::<_, StatRow>(
            "INSERT INTO stats (key, value, label) VALUES ($1, $2, $3) RETURNING *",
        )
        .bind(&key)
        .bind(body.value)
        .bind(&label)
        .fetch_one(&state.db)
        .await
        .map_err(unique_violation)?
    };

    Ok((
        StatusCode::CREATED,
        Json(json!({ "success": true, "message": "Stat created successfully", "data": stat_json_derived(&result) })),
    )
        .into_response())
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    _admin: AdminAuth,
    Path(key): Path<String>,
    Json(body): Json<UpdateStatRequest>,
) -> Result<Response, AppError> {
    let has_start = body
        .start_date
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    let result = if has_start {
        let start_date = parse_date(body.start_date)?;
        sqlx::query_as::<_, StatRow>(
            "UPDATE stats SET label = $1, start_date = $2, updated_at = CURRENT_TIMESTAMP WHERE key = $3 RETURNING *",
        )
        .bind(body.label)
        .bind(start_date)
        .bind(&key)
        .fetch_optional(&state.db)
        .await?
    } else {
        if body.value.is_none() {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({ "success": false, "message": "Value or start_date is required" })),
            )
                .into_response());
        }
        sqlx::query_as::<_, StatRow>(
            "UPDATE stats SET value = $1, label = $2, start_date = NULL, updated_at = CURRENT_TIMESTAMP WHERE key = $3 RETURNING *",
        )
        .bind(body.value)
        .bind(body.label)
        .bind(&key)
        .fetch_optional(&state.db)
        .await?
    };

    let Some(stat) = result else {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "message": "Stat not found" })),
        )
            .into_response());
    };
    Ok(Json(json!({ "success": true, "message": "Stat updated successfully", "data": stat_json_derived(&stat) }))
        .into_response())
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    _admin: AdminAuth,
    Path(key): Path<String>,
) -> Result<Response, AppError> {
    let res = sqlx::query("DELETE FROM stats WHERE key = $1")
        .bind(&key)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "message": "Stat not found" })),
        )
            .into_response());
    }
    Ok(Json(json!({ "success": true, "message": "Stat deleted successfully" })).into_response())
}

/// GET responses: total_projects reads live project count; start_date stats get
/// their value auto-computed (Node getStatByKey / getAllStats behavior).
fn stat_json(stat: &StatRow, project_count: i32) -> Value {
    let mut obj = serde_json::to_value(stat).unwrap().as_object().unwrap().clone();
    if stat.key == "total_projects" {
        obj.insert("value".into(), json!(project_count));
        obj.insert("calculated".into(), json!(true));
    } else if let Some(sd) = stat.start_date {
        obj.insert("value".into(), json!(calculate_months_diff(sd)));
        obj.insert("calculated".into(), json!(true));
    }
    Value::Object(obj)
}

/// Create/update responses: only start_date stats get a computed value (Node parity —
/// createStat/updateStat have no total_projects special case).
fn stat_json_derived(stat: &StatRow) -> Value {
    let mut obj = serde_json::to_value(stat).unwrap().as_object().unwrap().clone();
    if let Some(sd) = stat.start_date {
        obj.insert("value".into(), json!(calculate_months_diff(sd)));
        obj.insert("calculated".into(), json!(true));
    }
    Value::Object(obj)
}

fn unique_violation(e: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(db) = &e {
        if db.code().as_deref() == Some("23505") {
            return AppError::BadRequest("Stat with this key already exists".into());
        }
    }
    AppError::Internal(e.to_string())
}
