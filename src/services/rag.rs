use std::time::Duration;

use serde_json::{json, Value};

use crate::config::AppConfig;

/// Fire-and-forget hooks to the external RAG indexing service. All calls spawn a
/// background task; failures are logged, never propagated. The RAG service
/// reconciles its index against Postgres on startup, so a hook lost while it was
/// down is repaired on its next boot.
pub struct RagService {
    rag_url: String,
    secret: Option<String>,
    client: reqwest::Client,
}

impl RagService {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            rag_url: config.rag_service_url.clone(),
            secret: config.rag_api_secret.clone(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
        }
    }

    fn fire(&self, endpoint: &'static str, body: Value) {
        let client = self.client.clone();
        let url = format!("{}{}", self.rag_url, endpoint);
        let secret = self.secret.clone();
        tokio::spawn(async move {
            let mut req = client.post(&url).json(&body);
            if let Some(secret) = secret {
                req = req.header("X-RAG-Secret", secret);
            }
            match req.send().await {
                Ok(res) => tracing::debug!("[RAG Hook] {endpoint} -> {}", res.status()),
                Err(e) => tracing::warn!("[RAG Hook] {endpoint} error: {e}"),
            }
        });
    }

    pub fn on_project_created(&self, project: &Value) {
        self.fire("/index/add", project_doc(project));
        self.refresh_derived();
    }

    pub fn on_project_updated(&self, project: &Value) {
        self.fire("/index/update", project_doc(project));
        self.refresh_derived();
    }

    pub fn on_project_deleted(&self, project_id: i32) {
        self.fire("/index/delete", json!({ "chunk_id": format!("project_{project_id}") }));
        self.refresh_derived();
    }

    pub fn on_certificate_created(&self, cert: &Value) {
        self.fire("/index/add", certificate_doc(cert));
        self.refresh_derived();
    }

    pub fn on_certificate_updated(&self, cert: &Value) {
        self.fire("/index/update", certificate_doc(cert));
        self.refresh_derived();
    }

    pub fn on_certificate_deleted(&self, cert_id: i32) {
        self.fire("/index/delete", json!({ "chunk_id": format!("cert_{cert_id}") }));
        self.refresh_derived();
    }

    /// Ask the RAG service to recompute its whole-DB summary chunks (totals and
    /// the project list). It reads Postgres directly and owns the chunk text -
    /// this used to be assembled here from our own /api/stats, which quietly
    /// dropped the anti-hallucination guard the Python side writes.
    pub fn refresh_derived(&self) {
        self.fire("/index/refresh-derived", json!({}));
    }
}

/// Query the RAG service for context on the user's last message.
/// Returns `None` on any failure (graceful fallback). 3s timeout so a down RAG
/// never blocks the chat reply. `top_k` is deliberately omitted: the service
/// sizes it from the query.
pub async fn retrieve(client: &reqwest::Client, rag_url: &str, query: &str) -> Option<String> {
    let resp = client
        .post(format!("{rag_url}/retrieve"))
        .json(&json!({ "query": query }))
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let data: Value = resp.json().await.ok()?;
    if data.get("found").and_then(|f| f.as_bool()) == Some(true) {
        data.get("context").and_then(|c| c.as_str()).map(|s| s.to_string())
    } else {
        None
    }
}

fn project_doc(project: &Value) -> Value {
    let tags = project["tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|t| t.as_str()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    json!({
        "chunk_id": format!("project_{}", project["id"]),
        "text": format_project(project),
        "metadata": {
            "type": "project",
            "name": project["title"].as_str().unwrap_or(""),
            "title": project["title"].as_str().unwrap_or(""),
            "tags": tags,
            "db_id": project["id"].to_string(),
        },
    })
}

fn certificate_doc(cert: &Value) -> Value {
    json!({
        "chunk_id": format!("cert_{}", cert["id"]),
        "text": format_certificate(cert),
        "metadata": {
            "type": "certificate",
            "name": cert["title"].as_str().unwrap_or(""),
            "title": cert["title"].as_str().unwrap_or(""),
            "provider": cert["provider"].as_str().unwrap_or(""),
            "db_id": cert["id"].to_string(),
        },
    })
}

fn format_project(p: &Value) -> String {
    let tags = p["tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|t| t.as_str()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let links = p["links"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| {
                    let ty = l["type"].as_str().unwrap_or("");
                    let url = l["url"].as_str().unwrap_or("");
                    if ty.is_empty() || url.is_empty() {
                        None
                    } else {
                        Some(format!("{ty}: {url}"))
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    format!(
        "Project by Adit-san: {}. Description: {}. Tags/Tech stack: {}.{}",
        p["title"].as_str().unwrap_or(""),
        p["description"].as_str().unwrap_or(""),
        tags,
        if links.is_empty() { String::new() } else { format!(" Links: {links}") }
    )
}

fn format_certificate(c: &Value) -> String {
    let date = c["issue_date"]
        .as_str()
        .map(|d| d.chars().take(7).collect::<String>())
        .unwrap_or_else(|| "unknown date".into());
    let cred = c["credential_url"].as_str().unwrap_or("");
    format!(
        "Certificate earned by Adit-san: {}. Issued by: {}. Date: {}.{}",
        c["title"].as_str().unwrap_or(""),
        c["provider"].as_str().unwrap_or(""),
        date,
        if cred.is_empty() { String::new() } else { format!(" Credential: {cred}") }
    )
}
