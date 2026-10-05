// SPDX-License-Identifier: MPL-2.0

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};

pub const APP_ID: &str = "dev.korbeil.opencode-go-statusbar";
pub const ICON_NAME: &str = "dev.korbeil.opencode-go-statusbar-symbolic";

/// Which subscription an account authenticates against.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum Provider {
    /// An [OpenCode Go](https://opencode.ai/docs/go/) subscription, keyed by
    /// an API key from <https://opencode.ai/auth>.
    #[default]
    #[serde(rename = "opencode-go")]
    OpenCodeGo,
    /// A Claude (Pro/Max) subscription, keyed by the Claude Code OAuth token
    /// from `~/.claude/.credentials.json`.
    #[serde(rename = "claude")]
    Claude,
}

impl Provider {
    /// Human-readable name used in UI labels.
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenCodeGo => "OpenCode Go",
            Self::Claude => "Claude",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub key: String,
    /// Which subscription this account belongs to; defaults to
    /// `Provider::OpenCodeGo` when deserializing older configs.
    #[serde(default)]
    pub provider: Provider,
}

#[derive(Clone, Debug, CosmicConfigEntry, Eq, PartialEq)]
#[version = 1]
pub struct Config {
    pub accounts: Vec<Account>,
    pub refresh_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            accounts: Vec::new(),
            refresh_secs: 60,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        match cosmic_config::Config::new(APP_ID, Self::VERSION) {
            Ok(handler) => match Self::get_entry(&handler) {
                Ok(config) => config,
                Err((errors, config)) => {
                    for error in errors {
                        eprintln!("config load warning: {error}");
                    }
                    config
                }
            },
            Err(err) => {
                eprintln!("failed to open app config: {err}");
                Self::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_serializes_stable_names() {
        assert_eq!(
            serde_json::to_string(&Provider::OpenCodeGo).unwrap(),
            r#""opencode-go""#
        );
        assert_eq!(
            serde_json::to_string(&Provider::Claude).unwrap(),
            r#""claude""#
        );
    }

    #[test]
    fn accounts_without_provider_default_to_opencode_go() {
        let account: Account =
            serde_json::from_str(r#"{"name":"work","key":"sk-…"}"#).unwrap();
        assert_eq!(account.provider, Provider::OpenCodeGo);
    }
}
