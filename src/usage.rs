// SPDX-License-Identifier: MPL-2.0

use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::header::ACCEPT;
use reqwest::{Client, StatusCode};
use serde::Deserialize;

pub const USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const USER_AGENT: &str = concat!("opencode-go-statusbar/", env!("CARGO_PKG_VERSION"));

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
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKey(msg) => write!(f, "invalid API key: {msg}"),
            Self::NoSubscription(msg) => write!(f, "no OpenCode Go subscription: {msg}"),
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

pub async fn fetch_usage(client: &Client, api_key: &str) -> Result<Usage, FetchError> {
    let response = client
        .get(USAGE_URL)
        .bearer_auth(api_key)
        .header(ACCEPT, "application/json")
        .header("x-opencode-session", "opencode-go-statusbar")
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
}
