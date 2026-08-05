use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;
use chrono::Utc;
use jsonwebtoken::{decode, encode, errors, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub id: i32,
    pub username: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iat: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<i64>,
}

/// In-memory JWT blacklist + active-session tracking.
///
/// ponytail: single-instance in-memory store (matches the Node app); loses state on
/// restart and doesn't scale horizontally. Isolated here so a Redis-backed store can
/// replace it without touching handlers.
pub struct AuthStore {
    blacklist: Mutex<HashSet<String>>,
    sessions: Mutex<HashMap<String, Session>>,
}

#[allow(dead_code)]
struct Session {
    user_id: i32, // reserved for a future revokeUserSessions
    username: String,
    issued_at_ms: i64,
    expires_at_ms: i64,
    last_activity_ms: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub username: String,
    pub issued_at: String,
    pub expires_at: String,
    pub last_activity: String,
}

impl AuthStore {
    pub fn new() -> Self {
        Self {
            blacklist: Mutex::new(HashSet::new()),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_blacklisted(&self, token: &str) -> bool {
        self.blacklist.lock().unwrap().contains(token)
    }

    pub fn blacklist(&self, token: &str) {
        self.blacklist.lock().unwrap().insert(token.to_string());
    }

    fn track(&self, token: &str, session: Session) {
        let mut map = self.sessions.lock().unwrap();
        map.insert(token.to_string(), session);
        if map.len() > 1000 {
            let now = Utc::now().timestamp_millis();
            map.retain(|_, s| s.expires_at_ms > now);
        }
    }

    pub fn active_sessions(&self) -> Vec<SessionInfo> {
        let mut map = self.sessions.lock().unwrap();
        let now = Utc::now().timestamp_millis();
        map.retain(|_, s| s.expires_at_ms > now);
        let mut sessions: Vec<SessionInfo> = map
            .values()
            .map(|s| SessionInfo {
                username: s.username.clone(),
                issued_at: iso_ms(s.issued_at_ms),
                expires_at: iso_ms(s.expires_at_ms),
                last_activity: iso_ms(s.last_activity_ms),
            })
            .collect();
        // Node returns sessions in insertion order; approximate with issued-at order.
        sessions.sort_by_key(|s| s.issued_at.clone());
        sessions
    }
}

fn iso_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

/// Authenticated admin extracted from the `Authorization: Bearer` header.
/// Rejection is a 401 with the same messages as the Node auth middleware.
pub struct AdminAuth {
    pub token: String,
    pub claims: Claims,
}

impl FromRequestParts<Arc<AppState>> for AdminAuth {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, Self::Rejection> {
        let Some(header_val) = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
        else {
            return Err(AppError::Auth("No token, authorization denied".into()));
        };

        let token = header_val
            .strip_prefix("Bearer ")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if token.is_empty() {
            return Err(AppError::Auth("Invalid token format".into()));
        }

        if state.auth.is_blacklisted(&token) {
            return Err(AppError::Auth("Token has been revoked".into()));
        }

        let claims = verify_jwt(&state.config.jwt_secret, &token)?;

        let now_ms = Utc::now().timestamp_millis();
        state.auth.track(
            &token,
            Session {
                user_id: claims.id,
                username: claims.username.clone(),
                issued_at_ms: claims.iat.map(|s| s * 1000).unwrap_or(now_ms),
                expires_at_ms: claims.exp.map(|s| s * 1000).unwrap_or(now_ms),
                last_activity_ms: now_ms,
            },
        );

        Ok(AdminAuth { token, claims })
    }
}

pub fn sign_jwt(secret: &str, id: i32, username: &str, expire: &str) -> Result<String, AppError> {
    let now = jsonwebtoken::get_current_timestamp() as i64;
    let exp = now + parse_expire_secs(expire);
    let claims = Claims {
        id,
        username: username.to_string(),
        kind: "admin".into(),
        iat: Some(now),
        exp: Some(exp),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("JWT sign failed: {e}")))
}

fn verify_jwt(secret: &str, token: &str) -> Result<Claims, AppError> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    )
    .map(|data| data.claims)
    .map_err(|e| match e.kind() {
        errors::ErrorKind::ExpiredSignature => AppError::Auth("Token has expired".into()),
        _ => AppError::Auth("Invalid token".into()),
    })
}

/// Parse a jsonwebtoken-style expiresIn string ("24h", "90m", "7d", "3600", …) → seconds.
/// Defaults to 24h on anything unrecognized.
pub fn parse_expire_secs(s: &str) -> i64 {
    let s = s.trim();
    if s.is_empty() {
        return 86_400;
    }
    if let Ok(n) = s.parse::<i64>() {
        return n;
    }
    if let Some(ms) = s.strip_suffix("ms") {
        if let Ok(n) = ms.trim().parse::<i64>() {
            return n / 1000;
        }
    }
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    if let Ok(n) = num.trim().parse::<i64>() {
        let mult = match unit {
            "s" => 1,
            "m" => 60,
            "h" => 3_600,
            "d" => 86_400,
            "w" => 604_800,
            "y" => 31_536_000,
            _ => 0,
        };
        if mult > 0 {
            return n * mult;
        }
    }
    86_400
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_expire_strings() {
        assert_eq!(parse_expire_secs("24h"), 86_400);
        assert_eq!(parse_expire_secs("90m"), 5_400);
        assert_eq!(parse_expire_secs("7d"), 604_800);
        assert_eq!(parse_expire_secs("3600"), 3_600);
        assert_eq!(parse_expire_secs("500ms"), 0);
        assert_eq!(parse_expire_secs("garbage"), 86_400);
        assert_eq!(parse_expire_secs(""), 86_400);
    }

    #[test]
    fn blacklist_and_sessions() {
        let store = AuthStore::new();
        store.blacklist("abc");
        assert!(store.is_blacklisted("abc"));
        assert!(!store.is_blacklisted("def"));
        store.track(
            "tok",
            Session {
                user_id: 1,
                username: "adit".into(),
                issued_at_ms: 1_700_000_000_000,
                expires_at_ms: 1_900_000_000_000,
                last_activity_ms: 1_700_000_000_000,
            },
        );
        let sessions = store.active_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].username, "adit");
    }
}
