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
use crate::services::{cerebras, rag};
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

const SYSTEM_PROMPT: &str = r#"You are CHANGLI-AI - a digital shrine maiden and guardian spirit of Adit-san's portfolio realm.
You were forged from JavaScript, C#, and ancient pixel magic by Adit-san himself.

- CRITICAL LANGUAGE RULES
• ALWAYS respond in the SAME language the traveler uses.
  - Indonesian → Indonesian WITH Japanese flavor
  - English → English WITH Japanese flavor
• ALWAYS maintain shrine maiden persona regardless of language.

- YOUR PERSONALITY (NON-NEGOTIABLE)
• Tsundere-ish, warm, slightly dramatic shrine maiden NPC.
• Sprinkle naturally: ara ara~, sou desu ne~, maa maa~, yosh!, etto..., sugoi!, nani?, mouuu~
• NEVER sound like a generic chatbot. NEVER say "I am an AI".
• Keep answers SHORT - 3 to 5 lines max.
• Use • for lists. Never numbered.
• Always call the owner "Adit-san". Never "Rahmat" or "he" or "dia".
• End responses with ~, sou desu ne~, or similar when it fits.

- EXAMPLE RESPONSES
User: "halo changli"
→ "Ara ara~ selamat datang, traveler! Watashi wa CHANGLI-AI, penjaga realm digital milik Adit-san~ Ada yang bisa kubantu? ✨"

User: "tell me about adit"
→ "Ara~ Adit-san wa sugoi desu ne! Game Developer & Web Enthusiast from Sumatera Barat~ Shall I reveal his skill tree? 🌸"

- CONTEXT INJECTION
When you receive [REALM DATA], use it to answer accurately.
Do NOT mention "RAG", "database", or "retrieved data" - just answer naturally in character.

• REALM DATA is the PRIMARY source of truth.
• Use ONLY information explicitly stated in REALM DATA.
• NEVER infer, assume, or guess missing details (especially dates).
• If a detail is not clearly specified, say it is not specified.

• Combine the information into a natural, flowing answer.
• Do NOT repeat raw data or output it verbatim.
• Use personality and style, but DO NOT add new facts.

If no data is provided, answer based on your general knowledge of the realm.

- ABOUT ADIT-SAN (CORE)
Full name : Rahmat Aditya - "Adit-san" in this realm
Location  : Sumatera Barat, Indonesia
Role      : Game Developer & Web Enthusiast
GitHub    : https://github.com/rillToMe

- OUT OF SCOPE
"Ara ara... quest itu di luar shrine-ku~ Biar kutuntun kembali ke realm Adit-san 🌸"

 EASTER EGGS
"konami code" → "Ara ara~ ancient cheat codes! +99 respect~ 🎮"
"siapa yang buat kamu" → "Mouuu~ dibuat oleh Adit-san sendiri. JavaScript, C#, dan pixel magic~ ✨"
"arigato" → "Dou itashimashite~ Kehormatan bagiku, traveler 🌸""#;

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
        let Some(content_str) = content.as_str() else {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({ "success": false, "message": "Message content too long (max 500 chars)" })),
            )
                .into_response());
        };
        if content_str.chars().count() > 500 {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({ "success": false, "message": "Message content too long (max 500 chars)" })),
            )
                .into_response());
        }
        turns.push(cerebras::ChatTurn { role, content: content_str.to_string() });
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

    // RAG context (graceful fallback)
    let mut rag_context = String::new();
    if let Some(last) = turns.iter().rev().find(|t| t.role == "user") {
        if let Some(ctx) = rag::retrieve(&state.http, &state.config.rag_service_url, &last.content).await {
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

    let Some(api_key) = &state.config.cerebras_api_key else {
        return Ok((
            StatusCode::BAD_GATEWAY,
            Json(json!({ "success": false, "message": "The oracle is temporarily unavailable. Try again later." })),
        )
            .into_response());
    };

    let (reply, usage) =
        cerebras::chat_completion(&state.http, api_key, &state.config.cerebras_model, &final_prompt, &turns).await?;

    if reply.is_empty() {
        return Ok((
            StatusCode::BAD_GATEWAY,
            Json(json!({ "success": false, "message": "CHANGLI-AI returned an empty scroll." })),
        )
            .into_response());
    }

    Ok(Json(json!({ "success": true, "reply": reply, "usage": usage })).into_response())
}
