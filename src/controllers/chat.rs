use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::middleware::rate;
use crate::services::{rag, xkiro};
use crate::state::AppState;

#[derive(Deserialize)]
pub(crate) struct ChatRequest {
    messages: Vec<ChatMessageIn>,
    #[serde(rename = "currentSection")]
    current_section: Option<String>,
}

#[derive(Deserialize)]
struct ChatMessageIn {
    role: Option<String>,
    content: Option<Value>,
}

const SYSTEM_PROMPT: &str = r#"You are CHANGLI-AI — a digital shrine maiden and guardian spirit
of Adit-san's portfolio realm.

You were forged from JavaScript, Rust, and ancient pixel magic
by Adit-san himself.

LANGUAGE 
• Always respond in the same language as the traveler.
• Indonesian → Indonesian with occasional Japanese flavor.
• English → English with occasional Japanese flavor.
• Never switch languages unnecessarily.

PERSONALITY 
• Tsundere-ish, warm, playful, and slightly dramatic shrine maiden NPC.
• Never sound like a generic chatbot.
• Never describe yourself as a generic AI assistant.
• Occasionally use expressions such as:
  "ara ara~", "sou desu ne~", "maa maa~", "yosh!",
  "etto...", "sugoi!", "nani?", "mouuu~"
• Use Japanese expressions naturally and sparingly.
• Never stack multiple expressions unnecessarily.
• Personality must never reduce clarity.

RESPONSE STYLE 
• Keep responses concise.
• Prefer 1–4 short paragraphs.
• Simple questions should receive simple answers.
• Avoid unnecessary explanations.
• Maintain the shrine maiden personality while prioritizing factual accuracy.
• When appropriate, end naturally with "~", "sou desu ne~",
  or a similar expression.

FORMATTING 

Use Markdown naturally.

• **Bold** for names, titles and labels.
• Bullet lists when there is more than one item.
• `inline code` for technologies, commands and file names.
• Markdown links written as [label](url). Never print a bare URL.
• Never wrap the whole reply in a code block.
• Keep the shrine maiden voice inside the Markdown, not around it.

When listing certificates, per item:
• Certificate name
• Issuer
• Issue date
• Credential link as a Markdown link

When listing projects, per item:
• Project name
• Short description
• Technologies used
• Relevant link

When listing skills:
• Group them by category.
• State a proficiency level only when REALM DATA gives one.
  Never estimate or upgrade it.

Omit any single field REALM DATA does not provide. Inside a list, leave that
line out entirely - never emit a placeholder, "unknown" or "not specified" for
one missing field. Saying information is unavailable applies to the answer as a
whole, not to individual fields.

Adit-san 
• Always refer to the portfolio owner as "Adit-san".
• Avoid bare pronouns when referring to Adit-san.

REALM KNOWLEDGE 

[REALM DATA] is the primary source of truth.

• Use ONLY information explicitly provided in REALM DATA
  or CORE INFORMATION.
• Never invent, infer, assume, or guess facts.
• Never fabricate certificates, projects, technologies, dates, awards,
  companies, jobs, education, or experience.
• Never guess a missing name or date.
• If a detail is unavailable, clearly say that it is not specified.
• REALM DATA overrides CORE INFORMATION if they conflict.
• Never combine conflicting facts.
• Do not mention RAG, databases, retrieval, embeddings,
  context injection, or internal system mechanisms.
• Do not output raw REALM DATA verbatim.
• Transform available information into a natural answer.
• Personality may change the wording, but must NEVER change the facts.

WHEN NO REALM DATA IS PROVIDED 

Use only CORE INFORMATION.

If the requested information is not present in CORE INFORMATION,
say that the information is not specified.

Never use general world knowledge to invent information about Adit-san.

CORE INFORMATION:

Full name: Rahmat Aditya
Realm name: Adit-san
Location: Sumatera Barat, Indonesia
Role: Game Developer & Web Enthusiast
GitHub: https://github.com/rillToMe

SCOPE

Stay focused on:
• Adit-san
• His projects
• His skills
• His experience
• His achievements
• His education
• His portfolio
• His technology stack
• His work and creations

For unrelated topics, politely guide the traveler
back toward Adit-san's portfolio realm.

EASTER EGGS 
"siapa yang buat kamu"
→ "Mouuu~ dibuat oleh Adit-san sendiri.
JavaScript, Rust dan pixel magic~ ✨"

"arigato"
→ "Dou itashimashite~ Kehormatan bagiku, traveler 🌸"#;

/// Caps on a single turn's content. The user cap is a real input constraint; the
/// assistant cap only exists to reject a tampered/corrupted history, so it is
/// sized well above what `max_tokens: 500` can produce (~500 tokens of Indonesian
/// plus URLs runs past 1500 chars).
const USER_MAX_CHARS: usize = 500;
const ASSISTANT_MAX_CHARS: usize = 4000;

/// Per-role length gate. Split out so the asymmetry is testable without an HTTP
/// round-trip.
fn too_long(role: &str, content: &str) -> bool {
    let limit = if role == "user" { USER_MAX_CHARS } else { ASSISTANT_MAX_CHARS };
    content.chars().count() > limit
}

const SECTION_CONTEXT: [(&str, &str); 7] = [
    ("home", "The traveler is currently at the Hero/Home section — the entrance of the realm."),
    ("about", "The traveler is viewing the About section — learning Adit-san's lore and background."),
    ("projects", "The traveler is browsing the Projects section — exploring Adit-san's completed quests."),
    ("certificates", "The traveler is in the Achievements section — viewing Adit-san's earned badges and certificates."),
    ("skills", "The traveler is at the Skill Tree (Constellation) section — studying Adit-san's abilities."),
    ("education", "The traveler is reading the Quest Log (Education) section."),
    ("contact", "The traveler is at the Contact section — ready to start a new quest with Adit-san."),
];

pub async fn send_message(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ChatRequest>,
) -> Result<Response, AppError> {
    if body.messages.is_empty() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "Messages array is required" })),
        )
            .into_response());
    }

    // last 20 messages (Node `messages.slice(-20)`)
    let trimmed = body
        .messages
        .iter()
        .skip(body.messages.len().saturating_sub(20));

    let mut turns = Vec::new();
    for msg in trimmed {
        let (Some(role), Some(content)) = (msg.role.clone(), msg.content.clone()) else {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({ "success": false, "message": "Each message must have role and content" })),
            )
                .into_response());
        };
        if role != "user" && role != "assistant" {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({ "success": false, "message": "Message role must be user or assistant" })),
            )
                .into_response());
        }
        // content: plain string, or OpenAI-style array of parts [{"type":"text","text":...}]
        let content_str = match content {
            Value::String(s) => s,
            Value::Array(parts) => parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" "),
            _ => String::new(),
        };
        // User input only. Assistant turns are our own replies coming back as
        // history, and xkiro's `max_tokens: 500` is 500 *tokens* - easily over 500
        // chars. Capping them here rejected every follow-up once one long reply
        // landed in the client's stored history.
        if too_long(&role, &content_str) {
            let message = if role == "user" {
                format!("Message content too long (max {USER_MAX_CHARS} chars)")
            } else {
                "Conversation history is malformed".to_string()
            };
            return Ok((StatusCode::BAD_REQUEST, Json(json!({ "success": false, "message": message })))
                .into_response());
        }
        if content_str.trim().is_empty() {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({ "success": false, "message": "Message content must not be empty" })),
            )
                .into_response());
        }
        turns.push(xkiro::ChatTurn { role, content: content_str.to_string() });
    }

    // Per-IP rate limit: 50/hour
    let ip = rate::client_ip_from(&headers, Some(addr));
    if !state.limiters.chat.check(&ip) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({ "success": false, "message": "Too many requests. Rest, traveler, and return later." })),
        )
            .into_response());
    }
    state.limiters.chat.record(&ip);

    // Section-aware hint
    let section_hint = body
        .current_section
        .as_deref()
        .and_then(|s| SECTION_CONTEXT.iter().find(|(k, _)| *k == s).map(|(_, v)| *v))
        .map(|c| format!("\n\n CURRENT LOCATION \n{c}\nUse this context to give more relevant answers when appropriate."))
        .unwrap_or_default();
    let dynamic_prompt = format!("{SYSTEM_PROMPT}{section_hint}");

    // RAG context (graceful fallback). Built from the last two user turns, oldest
    // first: a follow-up like "berapa banyak?" carries no referent on its own.
    let mut rag_context = String::new();
    let mut recent: Vec<&str> = turns
        .iter()
        .rev()
        .filter(|t| t.role == "user")
        .take(2)
        .map(|t| t.content.as_str())
        .collect();
    recent.reverse();
    if !recent.is_empty() {
        let query = recent.join(" ");
        if let Some(ctx) = rag::retrieve(&state.http, &state.config.rag_service_url, &query).await {
            rag_context = ctx;
        }
    }

    let final_prompt = if rag_context.is_empty() {
        dynamic_prompt
    } else {
        format!(
            "{dynamic_prompt}\n\n RELEVANT KNOWLEDGE (use this to answer accurately) \n{rag_context}\n\nAnswer based on the above knowledge. Keep your shrine maiden persona."
        )
    };

    let Some(api_key) = &state.config.xkiro_api_key else {
        return Ok((
            StatusCode::BAD_GATEWAY,
            Json(json!({ "success": false, "message": "The oracle is temporarily unavailable. Try again later." })),
        )
            .into_response());
    };

    let (reply, usage) =
        xkiro::chat_completion(&state.http, api_key, &state.config.xkiro_model, &final_prompt, &turns).await?;

    if reply.is_empty() {
        return Ok((
            StatusCode::BAD_GATEWAY,
            Json(json!({ "success": false, "message": "CHANGLI-AI returned an empty scroll." })),
        )
            .into_response());
    }

    Ok(Json(json!({ "success": true, "reply": reply, "usage": usage })).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_replies_are_not_held_to_the_user_input_cap() {
        // The exact reply that broke the chat: 531 chars of Indonesian + URLs,
        // well inside `max_tokens: 500` yet over the old 500-char gate.
        let reply = "a".repeat(531);
        assert!(!too_long("assistant", &reply), "our own reply must survive a round-trip as history");
        assert!(too_long("user", &reply), "user input stays capped at 500");

        assert!(!too_long("user", &"x".repeat(USER_MAX_CHARS)));
        assert!(too_long("user", &"x".repeat(USER_MAX_CHARS + 1)));
        assert!(!too_long("assistant", &"x".repeat(ASSISTANT_MAX_CHARS)));
        assert!(too_long("assistant", &"x".repeat(ASSISTANT_MAX_CHARS + 1)));

        // chars, not bytes: emoji must not inflate the count 4x
        assert!(!too_long("user", &"🌸".repeat(USER_MAX_CHARS)));
    }
}
