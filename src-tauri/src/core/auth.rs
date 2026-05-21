use std::fs;
use std::path::Path;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::Value;

pub fn auth_identity_display_name(codex_home: &Path) -> Option<String> {
    let claims = auth_id_token_claims(codex_home);
    let display = claim_text(claims.get("email"))
        .or_else(|| claim_text(claims.get("name")))
        .or_else(|| claim_text(claims.get("preferred_username")))?;
    let plan = plan_label(auth_plan_type(codex_home).as_deref());
    if plan == "计划未知" {
        Some(display)
    } else {
        Some(format!("{display} · {plan}"))
    }
}

pub fn auth_identity_match_key(codex_home: &Path) -> Option<String> {
    let raw = read_auth_json(codex_home)?;
    if let Some(api_key) = api_key_match_key(&raw) {
        return Some(api_key);
    }
    let tokens = raw.get("tokens")?.as_object()?;
    let account_id = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let claims = auth_id_token_claims(codex_home);
    let subject = claim_text(claims.get("sub"));
    match (account_id, subject) {
        (Some(account_id), Some(subject)) => Some(format!("{account_id}:{subject}")),
        (Some(account_id), None) => Some(account_id.to_string()),
        (None, Some(subject)) => Some(subject),
        (None, None) => None,
    }
}

fn api_key_match_key(raw: &Value) -> Option<String> {
    let api_key = ["OPENAI_API_KEY", "openai_api_key", "api_key"]
        .iter()
        .find_map(|field| raw.get(field).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let auth_mode = raw
        .get("auth_mode")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase());
    if auth_mode.as_deref().is_some_and(|mode| mode == "apikey") || raw.get("tokens").is_none() {
        Some(format!("api-key:{api_key}"))
    } else {
        None
    }
}

pub fn unique_identity_name<'a>(
    base_name: &str,
    reserved: impl Iterator<Item = &'a str>,
) -> String {
    let reserved = reserved.collect::<Vec<_>>();
    let name = if base_name.trim().is_empty() {
        "账号".to_string()
    } else {
        base_name.trim().to_string()
    };
    if !reserved.iter().any(|existing| *existing == name) {
        return name;
    }
    let mut index = 2;
    loop {
        let candidate = format!("{name} {index}");
        if !reserved.iter().any(|existing| *existing == candidate) {
            return candidate;
        }
        index += 1;
    }
}

pub fn has_local_auth(codex_home: &Path) -> bool {
    codex_home.join("auth.json").exists()
}

pub fn auth_plan_type(codex_home: &Path) -> Option<String> {
    let raw = read_auth_json(codex_home)?;
    let tokens = raw.get("tokens")?.as_object()?;
    for token_name in ["access_token", "id_token"] {
        let claims = jwt_payload(tokens.get(token_name));
        let auth_claims = claims
            .get("https://api.openai.com/auth")
            .and_then(Value::as_object);
        if let Some(plan) = auth_claims
            .and_then(|claims| claims.get("chatgpt_plan_type"))
            .and_then(Value::as_str)
        {
            return Some(plan.to_string());
        }
    }
    None
}

pub fn plan_label(plan_type: Option<&str>) -> String {
    let Some(plan_type) = plan_type else {
        return "计划未知".to_string();
    };
    let normalized = plan_type.to_ascii_lowercase();
    if normalized.contains("business")
        || matches!(normalized.as_str(), "enterprise" | "enterprise_plus")
    {
        return "企业版".to_string();
    }
    if matches!(normalized.as_str(), "team" | "teams") {
        return "团队版".to_string();
    }
    if matches!(normalized.as_str(), "pro" | "plus") {
        return "个人版".to_string();
    }
    if normalized == "free" {
        return "免费版".to_string();
    }
    plan_type.to_string()
}

fn auth_id_token_claims(codex_home: &Path) -> serde_json::Map<String, Value> {
    let Some(raw) = read_auth_json(codex_home) else {
        return serde_json::Map::new();
    };
    let Some(tokens) = raw.get("tokens").and_then(Value::as_object) else {
        return serde_json::Map::new();
    };
    jwt_payload(tokens.get("id_token"))
        .as_object()
        .cloned()
        .unwrap_or_default()
}

fn read_auth_json(codex_home: &Path) -> Option<Value> {
    let raw = fs::read_to_string(codex_home.join("auth.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

fn jwt_payload(token: Option<&Value>) -> Value {
    let Some(token) = token.and_then(Value::as_str) else {
        return Value::Object(serde_json::Map::new());
    };
    if token.matches('.').count() < 2 {
        return Value::Object(serde_json::Map::new());
    }
    let payload = token.split('.').nth(1).unwrap_or_default();
    let Ok(decoded) = URL_SAFE_NO_PAD.decode(payload) else {
        return Value::Object(serde_json::Map::new());
    };
    serde_json::from_slice(&decoded).unwrap_or_else(|_| Value::Object(serde_json::Map::new()))
}

fn claim_text(value: Option<&Value>) -> Option<String> {
    let text = match value {
        Some(Value::String(value)) => value.trim().to_string(),
        Some(value) => value.to_string().trim_matches('"').trim().to_string(),
        None => String::new(),
    };
    (!text.is_empty()).then_some(text)
}
