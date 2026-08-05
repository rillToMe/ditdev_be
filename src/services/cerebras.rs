use serde_json::{json, Value};

use crate::error::AppError;

pub struct ChatTurn {
    pub role: String,
    pub content: String,
}

/// Call the Cerebras chat-completions API (OpenAI-compatible), mirroring
/// chatController.js: model from config, max_tokens 500, temperature 0.75.
/// Returns `(reply, usage)`. Non-2xx → 502; empty/parse failures → AppError.
pub async fn chat_completion(
    http: &reqwest::Client,
    api_key: &str,
    model: &str,
    system: &str,
    history: &[ChatTurn],
) -> Result<(String, Value), AppError> {
    let mut messages = Vec::with_capacity(history.len() + 1);
    messages.push(json!({ "role": "system", "content": system }));
    for m in history {
        messages.push(json!({ "role": m.role, "content": m.content }));
    }

    let resp = http
        .post("https://api.cerebras.ai/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&json!({
            "model": model,
            "max_tokens": 500,
            "temperature": 0.75,
            "messages": messages,
        }))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Cerebras request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(AppError::BadGateway(
            "The oracle is temporarily unavailable. Try again later.".into(),
        ));
    }

    let data: Value = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Cerebras response parse failed: {e}")))?;

    let reply = data["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();
    Ok((reply, data.get("usage").cloned().unwrap_or(Value::Null)))
}
