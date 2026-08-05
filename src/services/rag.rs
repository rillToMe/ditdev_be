use std::time::Duration;

use serde_json::{json, Value};

use crate::config::AppConfig;

/// Fire-and-forget hooks to the external RAG indexing service, mirroring
/// `services/ragIndexHooks.js`. All calls spawn a background task; failures are
/// logged, never propagated (parity with the Node fire-and-forget behavior).
pub struct RagService {
    rag_url: String,
    self_base: String,
    client: reqwest::Client,
}

impl RagService {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            rag_url: config.rag_service_url.clone(),
            self_base: format!("http://localhost:{}", config.port),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
        }
    }

    fn fire(&self, endpoint: &'static str, body: Value) {
        let client = self.client.clone();
        let url = format!("{}{}", self.rag_url, endpoint);
        tokio::spawn(async move {
            match client.post(&url).json(&body).send().await {
                Ok(res) => tracing::debug!("[RAG Hook] {endpoint} -> {}", res.status()),
                Err(e) => tracing::warn!("[RAG Hook] {endpoint} error: {e}"),
            }
        });
    }

    pub fn on_project_created(&self, project: &Value) {
        self.fire("/index/add", project_doc(project));
        self.sync_stats_chunk();
    }

    pub fn on_project_updated(&self, project: &Value) {
        self.fire("/index/update", project_doc(project));
    }

    pub fn on_project_deleted(&self, project_id: i32) {
        self.fire("/index/delete", json!({ "chunk_id": format!("project_{project_id}") }));
        self.sync_stats_chunk();
    }

    pub fn on_certificate_created(&self, cert: &Value) {
        self.fire("/index/add", certificate_doc(cert));
        self.sync_stats_chunk();
    }

    pub fn on_certificate_updated(&self, cert: &Value) {
        self.fire("/index/update", certificate_doc(cert));
    }

    pub fn on_certificate_deleted(&self, cert_id: i32) {
        self.fire("/index/delete", json!({ "chunk_id": format!("cert_{cert_id}") }));
        self.sync_stats_chunk();
    }

    /// Fetch the app's own /api/stats and /api/certificates, then update the RAG
    /// `stats_summary` chunk. Node does this with a 3s timeout per call.
    pub fn sync_stats_chunk(&self) {
        let client = self.client.clone();
        let stats_url = format!("{}/api/stats", self.self_base);
        let certs_url = format!("{}/api/certificates", self.self_base);
        let rag_url = format!("{}/index/update", self.rag_url);

        tokio::spawn(async move {
            let stats = client.get(&stats_url).send().await.ok()?.json::<Value>().await.ok()?;
            if stats.get("success").and_then(|s| s.as_bool()) != Some(true) {
                return None;
            }
            let data = stats.get("data").cloned().unwrap_or(Value::Null);
            let find = |key: &str| {
                data.as_array()?
                    .iter()
                    .find(|s| s.get("key").and_then(|k| k.as_str()) == Some(key))?
                    .get("value")
                    .cloned()
            };
            let total_projects = find("total_projects").and_then(|v| v.as_i64());
            let months = find("months_studying").and_then(|v| v.as_i64());

            let certs = client.get(&certs_url).send().await.ok()?;
            let total_certs = certs
                .json::<Value>()
                .await
                .ok()
                .and_then(|v| v.get("count").and_then(|c| c.as_u64()))
                .unwrap_or(0);

            let tp = total_projects.map(|n| n.to_string()).unwrap_or_else(|| "?".into());
            let mo = months.map(|n| n.to_string()).unwrap_or_default();
            let text = format!(
                "Adit-san's portfolio stats (real-time): Total projects completed: {tp}. Total certificates earned: {total_certs}.{}",
                if mo.is_empty() {
                    String::new()
                } else {
                    format!(" Months studying/coding: {mo} months.")
                }
            );

            let body = json!({
                "chunk_id": "stats_summary",
                "text": text,
                "metadata": {
                    "type": "stats",
                    "total_projects": tp,
                    "total_certs": total_certs.to_string(),
                    "months_studying": mo,
                },
            });
            let _ = client.post(&rag_url).json(&body).send().await;
            Some(())
        });
    }
}

/// Query the RAG service for context on the user's last message.
/// Returns `None` on any failure (graceful fallback, parity with the Node chat
/// controller). 3s timeout so a down RAG never blocks the chat reply.
pub async fn retrieve(client: &reqwest::Client, rag_url: &str, query: &str) -> Option<String> {
    let resp = client
        .post(format!("{rag_url}/retrieve"))
        .json(&json!({ "query": query, "top_k": 4 }))
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
    json!({
        "chunk_id": format!("project_{}", project["id"]),
        "text": format_project(project),
        "metadata": {
            "type": "project",
            "title": project["title"].as_str().unwrap_or(""),
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
            "title": cert["title"].as_str().unwrap_or(""),
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
