use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::config::AppConfig;
use crate::error::AppError;

/// 15 min server-side TTL so the browser-facing endpoints almost never hit GitHub's
/// unauthenticated 60 req/hr rate limit. The heatmap is additionally served with
/// `Cache-Control: max-age=3600` so the browser doesn't re-fetch on tab switches.
const CACHE_TTL: Duration = Duration::from_secs(15 * 60);

pub struct GitHubCache {
    inner: Mutex<Option<(Instant, Value)>>,
    heatmap: Mutex<Option<(Instant, Vec<u8>, String)>>, // (at, bytes, content_type)
}

impl GitHubCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
            heatmap: Mutex::new(None),
        }
    }

    fn get(&self) -> Option<Value> {
        let guard = self.inner.lock().unwrap();
        match &*guard {
            Some((at, v)) if at.elapsed() < CACHE_TTL => Some(v.clone()),
            _ => None,
        }
    }

    fn set(&self, v: Value) {
        *self.inner.lock().unwrap() = Some((Instant::now(), v));
    }

    fn get_heatmap(&self) -> Option<(Vec<u8>, String)> {
        let guard = self.heatmap.lock().unwrap();
        match &*guard {
            Some((at, body, ct)) if at.elapsed() < CACHE_TTL => Some((body.clone(), ct.clone())),
            _ => None,
        }
    }

    fn set_heatmap(&self, body: Vec<u8>, ct: String) {
        *self.heatmap.lock().unwrap() = Some((Instant::now(), body, ct));
    }
}

async fn gh_get(http: &reqwest::Client, url: &str, token: Option<&str>) -> Result<Value, AppError> {
    let mut req = http.get(url).header("User-Agent", "ditdev_be_rust");
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("GitHub request failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::BadGateway(format!("GitHub API error: {}", resp.status())));
    }
    resp.json()
        .await
        .map_err(|e| AppError::Internal(format!("GitHub parse failed: {e}")))
}

/// Fetch `{ events, user, repos }` for the configured user, served from cache when fresh.
pub async fn activity(
    http: &reqwest::Client,
    config: &AppConfig,
    cache: &GitHubCache,
) -> Result<Value, AppError> {
    if let Some(v) = cache.get() {
        return Ok(v);
    }

    let base = format!("https://api.github.com/users/{}", config.github_username);
    let events_url = format!("{base}/events/public?per_page=100");
    let repos_url = format!("{base}/repos?per_page=100&sort=updated");
    let token = config.github_token.as_deref();

    let (events, user, repos) = tokio::join!(
        gh_get(http, &events_url, token),
        gh_get(http, &base, token),
        gh_get(http, &repos_url, token),
    );

    let result = json!({
        "events": events?,
        "user": user?,
        "repos": repos?,
    });
    cache.set(result.clone());
    Ok(result)
}

/// Contribution heatmap image bytes, proxied so the browser caches it and stops
/// re-fetching from the external service on every tab switch. Falls back to the
/// activity-graph service if the primary ghchart source fails.
pub async fn heatmap(
    http: &reqwest::Client,
    config: &AppConfig,
    cache: &GitHubCache,
) -> Result<(Vec<u8>, String), AppError> {
    if let Some(body) = cache.get_heatmap() {
        return Ok(body);
    }

    let primary = format!(
        "https://ghchart.rshah.org/4f8cff/{}",
        config.github_username
    );
    let body = match fetch_bytes(http, &primary).await {
        Ok(b) => b,
        Err(_) => {
            let fallback = format!(
                "https://github-readme-activity-graph.vercel.app/graph?username={}&bg_color=0a0e1a&color=4f8cff&line=00d4ff&point=4f8cff&area=true&hide_border=true&theme=react-dark",
                config.github_username
            );
            fetch_bytes(http, &fallback).await?
        }
    };
    cache.set_heatmap(body.0.clone(), body.1.clone());
    Ok(body)
}

async fn fetch_bytes(http: &reqwest::Client, url: &str) -> Result<(Vec<u8>, String), AppError> {
    let resp = http
        .get(url)
        .header("User-Agent", "ditdev_be_rust")
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("heatmap fetch failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::BadGateway(format!(
            "heatmap source error: {}",
            resp.status()
        )));
    }
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/svg+xml")
        .to_string();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Internal(format!("heatmap read failed: {e}")))?
        .to_vec();
    Ok((bytes, ct))
}
