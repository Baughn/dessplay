//! The AI short-title curator: turns a series' pile of AniDB titles
//! into the one short name the fan community actually uses.
//!
//! The titles dump's kind-3 rows are *search tags*, not display names —
//! lowercase ("gochiusa"), season-suffixed ("gochiusa s2"), or opaque
//! ("s;g", "HnNKn") — and only ~a quarter of series have one at all.
//! No string heuristic recovers "Steins;Gate" from "s;g"; a language
//! model knows the community name outright. So the worker sends each
//! series' full title rows to the Anthropic API once, and caches the
//! answer durably in SQLite (`ai_short_titles`) — the API is consulted
//! once per series, ever, and the reconcile pass stays deterministic.
//!
//! The answer is **trusted as returned** (user decision 2026-08-18):
//! no grounding filter against the dump, because the community name is
//! sometimes absent from AniDB entirely. The backstop is display-side —
//! a human-edited entry name always wins over the curated title — plus
//! the ordinary edit paths.
//!
//! The API token is client-provisioned over the wire
//! (`ServerControl::SetAnthropicToken`) and read from the kv table per
//! call, so it can appear, rotate, or vanish at runtime without a
//! server restart. No token → the curator idles; nothing breaks.

use std::time::Duration;

use dessplay_core::types::AniDbSeriesId;

use crate::storage::TitleRow;

/// The model every call uses. Current Opus tier; the task is a trivial
/// knowledge lookup, so `effort: low` keeps it cheap and fast.
const MODEL: &str = "claude-opus-5";
/// The Messages endpoint.
const API_URL: &str = "https://api.anthropic.com/v1/messages";
/// Whole-request timeout. Batches are small and effort is low, but
/// thinking is on by default on this model tier — leave headroom.
const HTTP_TIMEOUT: Duration = Duration::from_secs(120);
/// Output cap. Thinking tokens count against it on this model; the
/// visible JSON is tiny.
const MAX_TOKENS: u32 = 16_000;
/// Sanity cap on an accepted short name — anything longer is not a
/// "short name" and is treated as no-answer.
const MAX_SHORT_LEN: usize = 64;

/// One series' worth of raw material for the model.
#[derive(Clone, Debug)]
pub struct CurationInput {
    /// The series being named.
    pub series: AniDbSeriesId,
    /// Every dump row for it (all kinds, all languages).
    pub rows: Vec<TitleRow>,
}

/// One curated answer. `None` = no community short name exists (a
/// durable answer, cached like any other).
pub type CuratedTitle = (AniDbSeriesId, Option<String>);

/// The model seam. Blocking — call from `spawn_blocking`, like
/// [`super::titles::TitlesSource`]. Errors are strings for logging;
/// the worker backs off and retries, never caches a failure.
pub trait ShortTitleCurator: Send + Sync + 'static {
    /// Curate one batch. Series missing from the reply are simply not
    /// cached (retried on a later pass); extras are ignored.
    fn curate(&self, token: &str, batch: &[CurationInput]) -> Result<Vec<CuratedTitle>, String>;
}

/// The real client. Deliberately no `Debug` impl and no stored token —
/// the token arrives per call, straight from the kv table.
pub struct AnthropicCurator {
    agent: ureq::Agent,
}

impl Default for AnthropicCurator {
    fn default() -> Self {
        Self::new()
    }
}

impl AnthropicCurator {
    /// A client; the token comes per call.
    pub fn new() -> Self {
        Self {
            agent: ureq::Agent::from(
                ureq::config::Config::builder()
                    .timeout_global(Some(HTTP_TIMEOUT))
                    // 4xx bodies name the offending field; surface them.
                    .http_status_as_error(false)
                    .build(),
            ),
        }
    }
}

impl ShortTitleCurator for AnthropicCurator {
    fn curate(&self, token: &str, batch: &[CurationInput]) -> Result<Vec<CuratedTitle>, String> {
        let body =
            serde_json::to_vec(&request_body(batch)).map_err(|e| format!("encoding: {e}"))?;
        let response = self
            .agent
            .post(API_URL)
            .header("x-api-key", token)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .send(&body[..])
            .map_err(|e| format!("http: {e}"))?;
        let status = response.status();
        let bytes = response
            .into_body()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| format!("reading response: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "status {}: {}",
                status.as_u16(),
                String::from_utf8_lossy(&bytes[..bytes.len().min(500)])
            ));
        }
        let reply: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("parsing response: {e}"))?;
        let usage = &reply["usage"];
        tracing::info!(
            batch = batch.len(),
            input_tokens = usage["input_tokens"].as_u64().unwrap_or(0),
            output_tokens = usage["output_tokens"].as_u64().unwrap_or(0),
            "curator: token usage"
        );
        parse_reply(&reply)
    }
}

/// The Messages request for one batch: structured output pinning the
/// exact reply shape, low effort (a knowledge lookup, not a reasoning
/// task), no sampling parameters (removed on this model tier).
fn request_body(batch: &[CurationInput]) -> serde_json::Value {
    serde_json::json!({
        "model": MODEL,
        "max_tokens": MAX_TOKENS,
        "output_config": {
            "effort": "low",
            "format": {
                "type": "json_schema",
                "schema": {
                    "type": "object",
                    "properties": {
                        "results": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "aid": {"type": "integer"},
                                    "short": {"anyOf": [{"type": "string"}, {"type": "null"}]}
                                },
                                "required": ["aid", "short"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["results"],
                    "additionalProperties": false
                }
            }
        },
        "messages": [{"role": "user", "content": prompt(batch)}],
    })
}

/// The one prompt. Titles ride along so the model anchors on the right
/// series (franchises reuse names across seasons and spin-offs).
fn prompt(batch: &[CurationInput]) -> String {
    use std::fmt::Write;
    let mut text = String::from(
        "For each numbered anime series below, give the short name the \
         English-speaking fan community most commonly uses for it in \
         writing — the name someone would naturally type in a chat \
         message (like GochiUsa, KonoSuba, Oregairu, Frieren), with the \
         community's usual casing and punctuation.\n\
         \n\
         Rules:\n\
         - Return null when the primary or official title is already \
         what people commonly use (e.g. Steins;Gate, K-On!), or when no \
         established short name exists. A short name must be genuinely \
         more convenient than the full title.\n\
         - Never invent an abbreviation. Only return names in real \
         common use.\n\
         - For sequels and later seasons, return the franchise's common \
         name without season markers or numbering (the UI shows the \
         season separately).\n\
         - These are display names for a list UI: proper display \
         casing, not lowercase search tags.\n\
         \n\
         Series, each with its AniDB title rows \
         (kind 1 = primary, 2 = synonym, 3 = short, 4 = official):\n",
    );
    for input in batch {
        let _ = writeln!(text, "\naid {}:", input.series.0);
        for row in &input.rows {
            let _ = writeln!(text, "  {} {}: {}", row.kind, row.lang, row.title);
        }
    }
    text
}

/// Pull the curated pairs out of a Messages reply. Refusals and shape
/// surprises are errors (retried later, never cached); individual
/// answers are trimmed, length-capped, and empty-normalized to `None`.
fn parse_reply(reply: &serde_json::Value) -> Result<Vec<CuratedTitle>, String> {
    if reply["stop_reason"].as_str() == Some("refusal") {
        return Err("model refused".into());
    }
    let text = reply["content"]
        .as_array()
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|block| block["type"].as_str() == Some("text"))
        })
        .and_then(|block| block["text"].as_str())
        .ok_or("no text block in response")?;
    let parsed: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("parsing structured output: {e}"))?;
    let results = parsed["results"]
        .as_array()
        .ok_or("no results array in structured output")?;
    let mut out = Vec::new();
    for entry in results {
        let Some(aid) = entry["aid"].as_u64().and_then(|v| u32::try_from(v).ok()) else {
            continue;
        };
        let short = entry["short"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= MAX_SHORT_LEN)
            .map(str::to_string);
        out.push((AniDbSeriesId(aid), short));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn input(aid: u32, rows: &[(u8, &str, &str)]) -> CurationInput {
        CurationInput {
            series: AniDbSeriesId(aid),
            rows: rows
                .iter()
                .map(|&(kind, lang, title)| TitleRow {
                    series: AniDbSeriesId(aid),
                    kind,
                    lang: lang.into(),
                    title: title.into(),
                })
                .collect(),
        }
    }

    #[test]
    fn prompt_lists_every_series_with_its_rows() {
        let text = prompt(&[
            input(5391, &[(1, "x-jat", "Gochuumon wa Usagi Desuka?")]),
            input(9310, &[(3, "en", "OG"), (3, "x-jat", "Oregairu")]),
        ]);
        assert!(text.contains("aid 5391:"));
        assert!(text.contains("1 x-jat: Gochuumon wa Usagi Desuka?"));
        assert!(text.contains("aid 9310:"));
        assert!(text.contains("3 en: OG"));
    }

    #[test]
    fn request_pins_model_and_structured_output() {
        let body = request_body(&[input(1, &[(1, "x-jat", "A")])]);
        assert_eq!(body["model"], MODEL);
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert_eq!(body["output_config"]["effort"], "low");
        // Sampling parameters are removed on this model tier; sending
        // one is a 400. Assert we never do.
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn parse_reply_extracts_answers_and_normalizes() {
        let reply = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": r#"{"results": [
                {"aid": 5391, "short": "GochiUsa"},
                {"aid": 17617, "short": null},
                {"aid": 9310, "short": "  "},
                {"aid": 1, "short": "x"}
            ]}"#}],
        });
        let parsed = parse_reply(&reply).unwrap();
        assert_eq!(
            parsed,
            vec![
                (AniDbSeriesId(5391), Some("GochiUsa".into())),
                (AniDbSeriesId(17617), None),
                (AniDbSeriesId(9310), None), // whitespace normalizes to no-answer
                (AniDbSeriesId(1), Some("x".into())),
            ]
        );
    }

    #[test]
    fn parse_reply_rejects_refusals_and_garbage() {
        let refusal = serde_json::json!({"stop_reason": "refusal", "content": []});
        assert!(parse_reply(&refusal).unwrap_err().contains("refused"));

        let garbage = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "not json"}],
        });
        assert!(parse_reply(&garbage).is_err());
    }

    #[test]
    fn parse_reply_length_caps_absurd_answers() {
        let long = "x".repeat(MAX_SHORT_LEN + 1);
        let reply = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{"type": "text",
                "text": format!(r#"{{"results": [{{"aid": 1, "short": "{long}"}}]}}"#)}],
        });
        assert_eq!(parse_reply(&reply).unwrap(), vec![(AniDbSeriesId(1), None)]);
    }
}
