// SPDX-License-Identifier: MPL-2.0

use std::env;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::header::ACCEPT;
use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::config::Provider;

pub const USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";
pub const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Claude Code's credentials file, the default source of the Claude OAuth
/// token. `CLAUDE_CONFIG_DIR` overrides its directory (Claude Code's own env
/// override), so the file is `$CLAUDE_CONFIG_DIR/.credentials.json`.
pub const CLAUDE_CREDENTIALS_FILE: &str = ".credentials.json";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const USER_AGENT: &str = concat!("opencode-go-statusbar/", env!("CARGO_PKG_VERSION"));
/// The beta header Claude Code itself sends; without it the endpoint rejects
/// OAuth authentication entirely.
const CLAUDE_BETA_HEADER: &str = "oauth-2025-04-20";

/// Quota state for a single usage window (`5h`, weekly or monthly).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowQuota {
    /// Percent of the window's budget already used, clamped to `0.0..=100.0`.
    pub used_percent: f64,
    /// The server explicitly reports this window as `rate-limited`.
    pub rate_limited: bool,
    pub resets_at: Option<DateTime<Utc>>,
}

impl WindowQuota {
    pub fn remaining_percent(&self) -> f64 {
        (100.0 - self.used_percent).clamp(0.0, 100.0)
    }

    pub fn blocked(&self) -> bool {
        self.rate_limited || self.used_percent >= 100.0
    }
}

/// All usage windows reported for one `OpenCode Go` account.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    /// The rolling 5-hour window.
    pub rolling: Option<WindowQuota>,
    pub weekly: Option<WindowQuota>,
    pub monthly: Option<WindowQuota>,
}

impl Usage {
    fn windows(&self) -> [Option<WindowQuota>; 3] {
        [self.rolling, self.weekly, self.monthly]
    }

    /// Lowest remaining percent across all reported windows.
    pub fn worst_remaining(&self) -> Option<f64> {
        self.windows()
            .into_iter()
            .flatten()
            .map(|window| window.remaining_percent())
            .reduce(f64::min)
    }

    /// True when any window is exhausted or rate-limited.
    pub fn blocked(&self) -> bool {
        self.windows()
            .into_iter()
            .flatten()
            .any(|window| window.blocked())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FetchError {
    /// HTTP 401: the API key was rejected.
    InvalidKey(String),
    /// HTTP 403 with an `EntitlementError`: the key is valid but has no Go plan.
    NoSubscription(String),
    /// Any other non-success HTTP status.
    Http(String),
    /// The request could not be completed.
    Network(String),
    /// The response body could not be parsed.
    Parse(String),
    /// A Claude account has no usable OAuth token: none pasted, none found
    /// on disk, or the on-disk one was rejected as expired.
    NoToken(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKey(msg) => write!(f, "invalid API key: {msg}"),
            Self::NoSubscription(msg) => write!(f, "no OpenCode Go subscription: {msg}"),
            Self::NoToken(msg) => write!(f, "no usable Claude OAuth token: {msg}"),
            Self::Http(msg) => write!(f, "server error ({msg})"),
            Self::Network(msg) => write!(f, "network error: {msg}"),
            Self::Parse(msg) => write!(f, "unexpected response: {msg}"),
        }
    }
}

impl std::error::Error for FetchError {}

pub fn client() -> Client {
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .unwrap_or_default()
}

/// Fetches the usage windows of one account, whichever provider it uses.
pub async fn fetch_usage(
    client: &Client,
    provider: Provider,
    key: &str,
) -> Result<Usage, FetchError> {
    match provider {
        Provider::OpenCodeGo => fetch_go_usage(client, key).await,
        Provider::Claude => fetch_claude_usage(client, key).await,
    }
}

async fn fetch_go_usage(client: &Client, api_key: &str) -> Result<Usage, FetchError> {
    let response = client
        .get(USAGE_URL)
        .bearer_auth(api_key)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|err| FetchError::Network(err.to_string()))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| FetchError::Network(err.to_string()))?;

    if status == StatusCode::UNAUTHORIZED {
        return Err(FetchError::InvalidKey(
            error_message(&body).unwrap_or_else(|| "key rejected".to_string()),
        ));
    }
    if status == StatusCode::FORBIDDEN {
        return if is_entitlement_error(&body) {
            Err(FetchError::NoSubscription(
                error_message(&body).unwrap_or_else(|| "subscription required".to_string()),
            ))
        } else {
            Err(FetchError::Http(match error_message(&body) {
                Some(msg) => format!("{status}: {msg}"),
                None => status.to_string(),
            }))
        };
    }
    if !status.is_success() {
        return Err(FetchError::Http(match error_message(&body) {
            Some(msg) => format!("{status}: {msg}"),
            None => status.to_string(),
        }));
    }

    parse_usage(&body)
}

async fn fetch_claude_usage(client: &Client, pasted: &str) -> Result<Usage, FetchError> {
    // The OAuth access token expires within hours; Claude Code rotates it in
    // its credentials file. Unless a token was pasted explicitly, read the
    // file fresh on every fetch so the applet always sees the live token.
    let token = resolve_token(pasted)?;

    let response = client
        .get(CLAUDE_USAGE_URL)
        .bearer_auth(&token)
        .header(ACCEPT, "application/json")
        .header("anthropic-beta", CLAUDE_BETA_HEADER)
        .send()
        .await
        .map_err(|err| FetchError::Network(err.to_string()))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| FetchError::Network(err.to_string()))?;

    if status == StatusCode::UNAUTHORIZED {
        // The token is expired or revoked. A pasted token is long dead by
        // then; a file token normally works because Claude Code refreshes it
        // before each request, so 401 usually means Claude Code is logged
        // out.
        return Err(FetchError::NoToken(match error_message(&body) {
            Some(msg) => msg,
            None => "OAuth token rejected (run `claude login` to renew it)".to_string(),
        }));
    }
    if !status.is_success() {
        return Err(FetchError::Http(match error_message(&body) {
            Some(msg) => format!("{status}: {msg}"),
            None => status.to_string(),
        }));
    }

    parse_claude_usage(&body)
}

/// Picks the OAuth token for a Claude account: the pasted override when
/// non-empty, otherwise the access token from Claude Code's credentials file.
fn resolve_token(pasted: &str) -> Result<String, FetchError> {
    if !pasted.trim().is_empty() {
        return Ok(pasted.trim().to_string());
    }
    read_claude_token().ok_or_else(|| {
        FetchError::NoToken(
            "no OAuth token: paste one in the settings or log in with Claude Code (claude login)"
                .to_string(),
        )
    })
}

/// Reads the OAuth access token from Claude Code's credentials file
/// (`$CLAUDE_CONFIG_DIR/.credentials.json`, default `~/.claude/`).
fn credentials_path() -> PathBuf {
    let dir = env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map_or_else(home_dir, PathBuf::from);
    dir.join(CLAUDE_CREDENTIALS_FILE)
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// Parses the access token out of the credentials file. The file holds
/// `{"claudeAiOauth": {"access_token": ...}}` (older builds used
/// `accessToken`); both spellings parse.
fn parse_claude_token(body: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(body).ok()?;
    ["/claudeAiOauth/accessToken", "/claudeAiOauth/access_token"]
        .into_iter()
        .find_map(|path| root.pointer(path).and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

fn read_claude_token() -> Option<String> {
    let body = std::fs::read_to_string(credentials_path()).ok()?;
    parse_claude_token(&body)
}

pub fn parse_usage(body: &str) -> Result<Usage, FetchError> {
    let response: UsageResponse = serde_json::from_str(body)
        .map_err(|err| FetchError::Parse(format!("could not parse usage ({err})")))?;
    let Some(windows) = response.usage else {
        return Err(FetchError::Parse("response had no usage object".to_string()));
    };

    Ok(Usage {
        rolling: windows.rolling.map(RawWindow::into_quota),
        weekly: windows.weekly.map(RawWindow::into_quota),
        monthly: windows.monthly.map(RawWindow::into_quota),
    })
}

fn error_message(body: &str) -> Option<String> {
    let response: ErrorBody = serde_json::from_str(body).ok()?;
    response
        .error
        .and_then(|detail| detail.message)
        .filter(|message| !message.trim().is_empty())
}

fn is_entitlement_error(body: &str) -> bool {
    serde_json::from_str::<ErrorBody>(body)
        .ok()
        .and_then(|response| response.error)
        .and_then(|detail| detail.error_type)
        .is_some_and(|error_type| error_type == "EntitlementError")
}

/// Claude's OAuth usage endpoint: `utilization` is the *used* percent (same
/// semantics as `percent` in the Go endpoint), and there is no monthly
/// window. Unknown keys (experimental per-model buckets, billing block) are
/// ignored.
#[derive(Deserialize)]
struct ClaudeUsageResponse {
    five_hour: Option<ClaudeWindow>,
    seven_day: Option<ClaudeWindow>,
}

#[derive(Deserialize)]
struct ClaudeWindow {
    /// Percent of the window's limit already consumed, 0–100.
    utilization: Option<f64>,
    resets_at: Option<String>,
}

impl ClaudeWindow {
    fn into_quota(self) -> WindowQuota {
        WindowQuota {
            used_percent: self.utilization.unwrap_or(0.0).clamp(0.0, 100.0),
            // The endpoint carries no rate-limit flag; an exhausted window
            // reads as blocked through `used_percent == 100`.
            rate_limited: false,
            resets_at: self
                .resets_at
                .and_then(|at| DateTime::parse_from_rfc3339(&at).ok())
                .map(|at| at.with_timezone(&Utc)),
        }
    }
}

/// Parses the Claude `/api/oauth/usage` response into the shared `Usage`.
///
/// # Errors
/// Returns [`FetchError::Parse`] when the body is not JSON or carries none
/// of the known usage windows.
pub fn parse_claude_usage(body: &str) -> Result<Usage, FetchError> {
    let response: ClaudeUsageResponse = serde_json::from_str(body)
        .map_err(|err| FetchError::Parse(format!("could not parse usage ({err})")))?;

    if response.five_hour.is_none() && response.seven_day.is_none() {
        return Err(FetchError::Parse(
            "response had no five_hour or seven_day window".to_string(),
        ));
    }

    Ok(Usage {
        rolling: response.five_hour.map(ClaudeWindow::into_quota),
        weekly: response.seven_day.map(ClaudeWindow::into_quota),
        monthly: None,
    })
}

#[derive(Deserialize)]
struct UsageResponse {
    usage: Option<UsageWindows>,
}

#[derive(Deserialize)]
struct UsageWindows {
    rolling: Option<RawWindow>,
    weekly: Option<RawWindow>,
    monthly: Option<RawWindow>,
}

#[derive(Deserialize)]
struct RawWindow {
    status: Option<String>,
    percent: Option<f64>,
    #[serde(rename = "resetsAt")]
    resets_at: Option<String>,
}

impl RawWindow {
    fn into_quota(self) -> WindowQuota {
        let used = self.percent.unwrap_or(0.0).clamp(0.0, 100.0);
        WindowQuota {
            used_percent: used,
            rate_limited: self.status.as_deref() == Some("rate-limited"),
            resets_at: self
                .resets_at
                .and_then(|at| DateTime::parse_from_rfc3339(&at).ok())
                .map(|at| at.with_timezone(&Utc)),
        }
    }
}

#[derive(Deserialize)]
struct ErrorBody {
    error: Option<ErrorDetail>,
}

#[derive(Deserialize)]
struct ErrorDetail {
    #[serde(rename = "type")]
    error_type: Option<String>,
    message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_BODY: &str = r#"{
        "usage": {
            "rolling": { "status": "ok", "percent": 25.0, "resetsAt": "2026-09-03T02:15:00Z" },
            "weekly": { "status": "ok", "percent": 50.0, "resetsAt": "2026-09-05T00:00:00Z" },
            "monthly": { "status": "ok", "percent": 10.0, "resetsAt": "2026-10-01T00:00:00Z" }
        }
    }"#;

    const RATE_LIMITED_BODY: &str = r#"{
        "usage": {
            "rolling": { "status": "rate-limited", "percent": 100.0, "resetsAt": "2026-09-03T02:15:00Z" },
            "weekly": { "status": "ok", "percent": 12.0, "resetsAt": "2026-09-05T00:00:00Z" },
            "monthly": { "status": "ok", "percent": 8.0, "resetsAt": "2026-10-01T00:00:00Z" }
        }
    }"#;

    const PARTIAL_BODY: &str = r#"{
        "usage": { "weekly": { "status": "ok", "percent": 33.0, "resetsAt": "2026-09-05T00:00:00Z" } }
    }"#;

    const AUTH_ERROR_BODY: &str =
        r#"{"type":"error","error":{"type":"AuthError","message":"Missing API key."}}"#;
    const ENTITLEMENT_ERROR_BODY: &str = r#"{"type":"error","error":{"type":"EntitlementError","message":"OpenCode Go subscription required."}}"#;

    #[test]
    fn parses_full_usage() {
        let usage = parse_usage(FULL_BODY).unwrap();
        let rolling = usage.rolling.unwrap();
        assert!((rolling.used_percent - 25.0).abs() < f64::EPSILON);
        assert!((rolling.remaining_percent() - 75.0).abs() < f64::EPSILON);
        assert!(rolling.resets_at.is_some());
        assert!(!usage.blocked());
        assert!((usage.worst_remaining().unwrap() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parses_partial_usage() {
        let usage = parse_usage(PARTIAL_BODY).unwrap();
        assert!(usage.rolling.is_none());
        assert!(usage.monthly.is_none());
        assert!((usage.worst_remaining().unwrap() - 67.0).abs() < f64::EPSILON);
        assert!(!usage.blocked());
    }

    #[test]
    fn rate_limited_window_blocks() {
        let usage = parse_usage(RATE_LIMITED_BODY).unwrap();
        assert!(usage.blocked());
        assert!((usage.worst_remaining().unwrap() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn clamps_out_of_range_percent() {
        let usage = parse_usage(r#"{"usage":{"rolling":{"percent":150.0}}}"#).unwrap();
        assert_eq!(usage.rolling.unwrap().used_percent, 100.0);
        assert!(usage.blocked());

        let usage = parse_usage(r#"{"usage":{"rolling":{"percent":-10.0}}}"#).unwrap();
        assert_eq!(usage.rolling.unwrap().used_percent, 0.0);
    }

    #[test]
    fn missing_fields_default_gracefully() {
        let usage = parse_usage(r#"{"usage":{}}"#).unwrap();
        assert!(usage.worst_remaining().is_none());
        assert!(!usage.blocked());
    }

    #[test]
    fn rejects_empty_body() {
        assert!(matches!(parse_usage("{}"), Err(FetchError::Parse(_))));
        assert!(matches!(parse_usage("not json"), Err(FetchError::Parse(_))));
    }

    #[test]
    fn extracts_error_messages() {
        assert_eq!(
            error_message(AUTH_ERROR_BODY).as_deref(),
            Some("Missing API key.")
        );
        assert!(error_message(FULL_BODY).is_none());
        assert!(error_message("").is_none());
    }

    #[test]
    fn detects_entitlement_errors() {
        assert!(is_entitlement_error(ENTITLEMENT_ERROR_BODY));
        assert!(!is_entitlement_error(AUTH_ERROR_BODY));
        assert!(!is_entitlement_error(""));
    }

    const CLAUDE_FULL_BODY: &str = r#"{
        "five_hour": {
            "utilization": 37.0,
            "resets_at": "2026-03-10T04:59:59.000000+00:00"
        },
        "seven_day": {
            "utilization": 26.0,
            "resets_at": "2026-03-15T14:59:59.771647+00:00"
        },
        "seven_day_opus": null,
        "seven_day_sonnet": { "utilization": 1.0, "resets_at": null },
        "extra_usage": { "is_enabled": false, "monthly_limit": null }
    }"#;

    #[test]
    fn parses_claude_usage() {
        let usage = parse_claude_usage(CLAUDE_FULL_BODY).unwrap();
        let rolling = usage.rolling.unwrap();
        assert!((rolling.used_percent - 37.0).abs() < f64::EPSILON);
        assert!((rolling.remaining_percent() - 63.0).abs() < f64::EPSILON);
        assert!(rolling.resets_at.is_some());
        assert!(!rolling.rate_limited);

        let weekly = usage.weekly.unwrap();
        assert!((weekly.used_percent - 26.0).abs() < f64::EPSILON);
        assert!(weekly.resets_at.is_some());

        // Claude has no monthly window.
        assert!(usage.monthly.is_none());
        assert!(!usage.blocked());
        // Worst of 63% (5h) and 74% (weekly) remaining.
        assert!((usage.worst_remaining().unwrap() - 63.0).abs() < f64::EPSILON);
    }

    #[test]
    fn claude_full_window_blocks() {
        let usage = parse_claude_usage(r#"{"five_hour":{"utilization":100.0}}"#).unwrap();
        assert!(usage.blocked());
        assert!((usage.worst_remaining().unwrap()).abs() < f64::EPSILON);
    }

    #[test]
    fn claude_empty_windows_default_to_zero() {
        let usage = parse_claude_usage(r#"{"five_hour":{},"seven_day":{}}"#).unwrap();
        assert_eq!(usage.rolling.unwrap().used_percent, 0.0);
        assert_eq!(usage.weekly.unwrap().used_percent, 0.0);
        assert!(usage.rolling.unwrap().resets_at.is_none());
        assert!(!usage.blocked());
    }

    #[test]
    fn claude_clamps_out_of_range_utilization() {
        let usage = parse_claude_usage(r#"{"five_hour":{"utilization":130.0}}"#).unwrap();
        assert_eq!(usage.rolling.unwrap().used_percent, 100.0);
    }

    #[test]
    fn rejects_claude_body_without_windows() {
        assert!(matches!(parse_claude_usage("{}"), Err(FetchError::Parse(_))));
        assert!(matches!(
            parse_claude_usage(r#"{"extra_usage":{"is_enabled":false}}"#),
            Err(FetchError::Parse(_))
        ));
        assert!(matches!(parse_claude_usage("not json"), Err(FetchError::Parse(_))));
    }

    #[test]
    fn pasted_token_wins_over_file() {
        assert_eq!(
            resolve_token("  sk-ant-oat01-pasted  ").unwrap(),
            "sk-ant-oat01-pasted"
        );
    }

    #[test]
    fn parses_claude_credentials_variants() {
        assert_eq!(
            parse_claude_token(r#"{"claudeAiOauth":{"accessToken":"tok-a","refreshToken":"r"}}"#)
                .as_deref(),
            Some("tok-a")
        );
        assert_eq!(
            parse_claude_token(r#"{"claudeAiOauth":{"access_token":"tok-b"}}"#).as_deref(),
            Some("tok-b")
        );
        assert_eq!(
            parse_claude_token(r#"{"claudeAiOauth":{"accessToken":"  "}}"#),
            None
        );
        assert_eq!(parse_claude_token("{}"), None);
        assert_eq!(parse_claude_token("not json"), None);
    }
}
