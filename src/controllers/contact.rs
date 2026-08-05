use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::error::AppError;
use crate::state::AppState;

#[derive(Deserialize)]
pub(crate) struct ContactRequest {
    name: Option<String>,
    email: Option<String>,
    message: Option<String>,
}

pub async fn send_message(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ContactRequest>,
) -> Result<Response, AppError> {
    let (Some(name), Some(email), Some(message)) = (body.name, body.email, body.message) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "Name, email, and message are required" })),
        )
            .into_response());
    };
    if !is_valid_email(&email) {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "Invalid email format" })),
        )
            .into_response());
    }
    if message.chars().count() > 2000 {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "Message too long (max 2000 chars)" })),
        )
            .into_response());
    }

    let Some(webhook_url) = &state.config.discord_webhook_url else {
        tracing::error!("DISCORD_WEBHOOK_URL not set");
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "success": false, "message": "Server misconfiguration" })),
        )
            .into_response());
    };

    let payload = json!({
        "username": "Portfolio Bot",
        "avatar_url": "https://cdn.discordapp.com/embed/avatars/0.png",
        "embeds": [{
            "title": "📬 New Message from Portfolio",
            "color": 0x4f8cff,
            "fields": [
                { "name": "👤 Name", "value": name, "inline": true },
                { "name": "📧 Email", "value": email, "inline": true },
                { "name": "💬 Message", "value": message, "inline": false },
            ],
            "footer": { "text": "DitDev Portfolio" },
            "timestamp": chrono::Utc::now().to_rfc3339(),
        }],
    });

    let resp = state
        .http
        .post(webhook_url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Discord request failed: {e}")))?;

    if !resp.status().is_success() {
        tracing::error!("Discord webhook error: {}", resp.status());
        return Ok((
            StatusCode::BAD_GATEWAY,
            Json(json!({ "success": false, "message": "Failed to deliver message" })),
        )
            .into_response());
    }

    tracing::info!("Contact message sent to Discord");
    Ok(Json(json!({ "success": true, "message": "Message sent successfully" })).into_response())
}

/// Equivalent of the Node `^[^\s@]+@[^\s@]+\.[^\s@]+$` regex.
fn is_valid_email(email: &str) -> bool {
    if email.is_empty() || email.contains(char::is_whitespace) {
        return false;
    }
    let mut parts = email.split('@');
    let local = parts.next().unwrap_or("");
    let domain = parts.next().unwrap_or("");
    if local.is_empty() || domain.is_empty() || parts.next().is_some() {
        return false;
    }
    let Some(dot) = domain.find('.') else {
        return false;
    };
    dot > 0 && dot < domain.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_validation() {
        assert!(is_valid_email("adit@example.com"));
        assert!(is_valid_email("a.b+c@sub.domain.co"));
        assert!(!is_valid_email("no-at-sign"));
        assert!(!is_valid_email("a@b")); // no dot
        assert!(!is_valid_email("@b.com"));
        assert!(!is_valid_email("a@.com"));
        assert!(!is_valid_email("a@b."));
        assert!(!is_valid_email("a b@c.com"));
        assert!(!is_valid_email("a@b@c.com"));
    }
}
