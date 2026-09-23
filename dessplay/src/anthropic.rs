//! A minimal blocking Anthropic Messages client, shared by the client's
//! AI features (commentary, the oracle). Bodies are plain
//! `serde_json::Value`s built by each feature's pure request builder;
//! this module owns only the transport: headers, the response-size cap,
//! error-body extraction, and token-usage logging.
//!
//! Always called under `spawn_blocking` — `ureq` is synchronous.

use std::time::Duration;

use dessplay_core::ai::{ANTHROPIC_API_URL, ANTHROPIC_VERSION};

/// Largest response body we read. Replies are text; this only guards
/// against a runaway body.
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

/// A failed Messages call.
#[derive(Debug)]
pub enum ApiError {
    /// Transport failure or a non-2xx response (with the API's own
    /// error message when the body carried one).
    Http(String),
    /// A 2xx response we could not parse.
    Api(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Http(e) => write!(f, "http: {e}"),
            ApiError::Api(e) => write!(f, "api: {e}"),
        }
    }
}

/// The client. Deliberately no `Debug` impl — it holds the API token.
pub struct Anthropic {
    agent: ureq::Agent,
    token: String,
}

impl Anthropic {
    /// A client for `token` whose whole-request timeout is `timeout`.
    pub fn new(token: String, timeout: Duration) -> Self {
        Self {
            agent: ureq::Agent::from(
                ureq::config::Config::builder()
                    .timeout_global(Some(timeout))
                    // A 4xx must reach us as a response, not an error:
                    // the body names the offending field, and ureq's
                    // status-as-error discards it (a bare "http status:
                    // 400" was undiagnosable, 2026-07-26).
                    .http_status_as_error(false)
                    .build(),
            ),
            token,
        }
    }

    /// One Messages call; returns the parsed reply. `what` labels the
    /// token-usage log line (e.g. "commentary/cast", "oracle").
    pub fn messages(
        &self,
        body: &serde_json::Value,
        what: &str,
    ) -> Result<serde_json::Value, ApiError> {
        let bytes = serde_json::to_vec(body)
            .map_err(|e| ApiError::Api(format!("encoding request: {e}")))?;
        let response = self
            .agent
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.token)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .send(&bytes[..])
            .map_err(|e| ApiError::Http(e.to_string()))?;
        let status = response.status();
        let reply = response
            .into_body()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_vec()
            .map_err(|e| ApiError::Http(format!("reading response: {e}")))?;
        if !status.is_success() {
            return Err(ApiError::Http(format!(
                "status {}: {}",
                status.as_u16(),
                api_error_detail(&reply)
            )));
        }
        let reply: serde_json::Value = serde_json::from_slice(&reply)
            .map_err(|e| ApiError::Api(format!("parsing response: {e}")))?;
        // Token accounting at info: caching and tool use are invisible
        // otherwise, and "is the cache hitting?" is one grep away.
        let usage = &reply["usage"];
        tracing::info!(
            call = what,
            input_tokens = usage["input_tokens"].as_u64().unwrap_or(0),
            output_tokens = usage["output_tokens"].as_u64().unwrap_or(0),
            cache_read_tokens = usage["cache_read_input_tokens"].as_u64().unwrap_or(0),
            cache_write_tokens = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0),
            web_searches = usage["server_tool_use"]["web_search_requests"]
                .as_u64()
                .unwrap_or(0),
            web_fetches = usage["server_tool_use"]["web_fetch_requests"]
                .as_u64()
                .unwrap_or(0),
            stop_reason = reply["stop_reason"].as_str().unwrap_or(""),
            "anthropic: token usage"
        );
        Ok(reply)
    }
}

/// Whether the reply is a safety refusal (`stop_reason: refusal`).
pub fn is_refusal(reply: &serde_json::Value) -> bool {
    reply["stop_reason"].as_str() == Some("refusal")
}

/// The first `text` block of a reply. Thinking blocks come first on
/// current models, so position is never assumed.
pub fn first_text(reply: &serde_json::Value) -> Option<&str> {
    reply["content"]
        .as_array()?
        .iter()
        .find(|block| block["type"].as_str() == Some("text"))?["text"]
        .as_str()
}

/// Every `text` block of a reply, concatenated in order. Web-search
/// citations split one answer across several blocks, so a reply that
/// may have used search must be read this way, not by [`first_text`].
pub fn all_text(reply: &serde_json::Value) -> String {
    reply["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block["type"].as_str() == Some("text"))
                .filter_map(|block| block["text"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

/// Pull the human-readable message out of an Anthropic error body
/// (`{"type":"error","error":{"message":…}}`), falling back to a
/// truncated raw snippet — a 4xx must always say *why*, not just that
/// it happened.
pub fn api_error_detail(body: &[u8]) -> String {
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(body)
        && let Some(msg) = v["error"]["message"].as_str()
    {
        return msg.to_string();
    }
    String::from_utf8_lossy(body).chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An Anthropic 4xx body names the offending field; the logged
    /// error must carry that message, not a bare "http status: 400"
    /// (which is what made the original failure undiagnosable).
    #[test]
    fn api_error_details_are_extracted_from_the_body() {
        let body = br#"{"type":"error","error":{"type":"invalid_request_error","message":"image exceeds 10 MB maximum"}}"#;
        assert_eq!(api_error_detail(body), "image exceeds 10 MB maximum");
        assert_eq!(api_error_detail(b"not json at all"), "not json at all");
    }

    #[test]
    fn text_is_read_by_block_type_not_position() {
        let reply = serde_json::json!({
            "content": [
                {"type": "thinking", "thinking": ""},
                {"type": "server_tool_use", "name": "web_search"},
                {"type": "text", "text": "Yes, "},
                {"type": "text", "text": "red foxes live there.", "citations": []},
            ]
        });
        assert_eq!(first_text(&reply), Some("Yes, "));
        assert_eq!(all_text(&reply), "Yes, red foxes live there.");
        assert_eq!(all_text(&serde_json::json!({})), "");
        assert!(first_text(&serde_json::json!({"content": []})).is_none());
    }

    #[test]
    fn refusal_is_detected() {
        assert!(is_refusal(&serde_json::json!({"stop_reason": "refusal"})));
        assert!(!is_refusal(&serde_json::json!({"stop_reason": "end_turn"})));
    }
}
