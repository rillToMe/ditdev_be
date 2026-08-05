use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Datelike;
use rand::Rng;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::middleware::rate;
use crate::state::AppState;

const TICK_COOLDOWN_MS: i64 = 2000;

pub async fn get_xp(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let bonus = bonus_xp(&state.db).await?;
    let base = calc_base_xp();
    Ok(Json(json!({
        "success": true,
        "total_xp": base + bonus,
        "base_xp": base,
        "bonus_xp": bonus,
    })))
}

pub async fn tick_xp(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let ip = rate::client_ip_from(&headers, Some(addr));
    let now = chrono::Utc::now().timestamp_millis();

    let last = state.xp_ticks.lock().unwrap().get(&ip).copied().unwrap_or(0);
    if now - last < TICK_COOLDOWN_MS {
        // Rate limited: return current total without incrementing (no `gain` field).
        let bonus = bonus_xp(&state.db).await?;
        let base = calc_base_xp();
        return Ok(Json(json!({
            "success": true,
            "total_xp": base + bonus,
            "rate_limited": true,
        })));
    }

    {
        let mut ticks = state.xp_ticks.lock().unwrap();
        ticks.insert(ip.clone(), now);
        // ponytail: in-memory per-IP cooldowns, single instance; purge stale entries to bound memory
        if ticks.len() > 1000 {
            let cutoff = now - 60_000;
            ticks.retain(|_, ts| *ts >= cutoff);
        }
    }

    let gain = rand::thread_rng().gen_range(1..=4); // 1–4 XP, matches Math.random() floor+1
    let bonus: i64 = sqlx::query_scalar(
        "UPDATE xp_global SET bonus_xp = bonus_xp + $1, updated_at = NOW() WHERE id = 1 RETURNING bonus_xp",
    )
    .bind(gain)
    .fetch_one(&state.db)
    .await?;

    let base = calc_base_xp();
    Ok(Json(json!({
        "success": true,
        "total_xp": base + bonus,
        "gain": gain,
        "rate_limited": false,
    })))
}

async fn bonus_xp(pool: &sqlx::PgPool) -> Result<i64, AppError> {
    let bonus: Option<i64> = sqlx::query_scalar("SELECT bonus_xp FROM xp_global WHERE id = 1")
        .fetch_optional(pool)
        .await?;
    Ok(bonus.unwrap_or(0))
}

/// Deterministic daily XP from 2024-08-28 to today (UTC), port of `calcBaseXP`.
pub fn calc_base_xp() -> i64 {
    calc_base_xp_until(chrono::Utc::now().date_naive())
}

/// Testable core: iterate each day, seed mulberry32 with YYYYMMDD, roll tier then value.
pub fn calc_base_xp_until(today: chrono::NaiveDate) -> i64 {
    let start = chrono::NaiveDate::from_ymd_opt(2024, 8, 28).unwrap();
    let mut total = 0i64;
    let mut cursor = start;
    while cursor <= today {
        let seed = (cursor.year() * 10000 + cursor.month() as i32 * 100 + cursor.day() as i32) as u32;
        let mut rng = mulberry32(seed);
        let roll = rng();
        let xp = if roll < 0.20 {
            (rng() * 16.0).floor() as i32 + 5 // 5–20 "days off"
        } else if roll < 0.70 {
            (rng() * 31.0).floor() as i32 + 25 // 25–55 normal
        } else {
            (rng() * 41.0).floor() as i32 + 60 // 60–100 productive
        };
        total += xp as i64;
        cursor = cursor.succ_opt().unwrap();
    }
    total
}

/// Port of the mulberry32 seeded PRNG from xpController.js.
fn mulberry32(mut s: u32) -> impl FnMut() -> f64 {
    move || {
        s = s.wrapping_add(0x6D2B79F5);
        let mut t = imul(s ^ (s >> 15), s | 1);
        t = t.wrapping_add(imul(t ^ (t >> 7), t | 61));
        ((t ^ (t >> 14)) as f64) / 4_294_967_296.0
    }
}

/// `Math.imul` — signed 32-bit multiplication, bit pattern preserved as u32.
fn imul(a: u32, b: u32) -> u32 {
    (a as i32).wrapping_mul(b as i32) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_xp_matches_node_reference() {
        // Reference computed by running xpController.js calcBaseXP for 2024-08-28..2026-08-04.
        let end = chrono::NaiveDate::from_ymd_opt(2026, 8, 4).unwrap();
        assert_eq!(calc_base_xp_until(end), 32_612);
    }

    #[test]
    fn single_day_seed_deterministic() {
        // Same date → same XP (recompute a few times for determinism).
        let end = chrono::NaiveDate::from_ymd_opt(2024, 8, 28).unwrap();
        let a = calc_base_xp_until(end);
        let b = calc_base_xp_until(end);
        assert_eq!(a, b);
        // XP for a single day must be in 5..=100
        assert!((5..=100).contains(&a));
    }
}
