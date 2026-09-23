//! Anthropic API constants shared by every AI feature (commentary, the
//! short-title curator, the oracle). One model for the whole project, so
//! a model bump is one edit and features can't drift apart
//! (design.md, AI Features; decisions.md, One Anthropic model constant).

/// The model every Anthropic call uses. On this model thinking is
/// always on (`{type: "disabled"}` and `budget_tokens` are 400s) and
/// the default effort is `medium`, so every caller pins
/// `output_config.effort` explicitly and sizes `max_tokens` to cover
/// thinking as well as the reply.
pub const ANTHROPIC_MODEL: &str = "claude-opus-5-5";

/// The Messages endpoint.
pub const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";

/// The `anthropic-version` header value.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
