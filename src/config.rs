// SPDX-License-Identifier: MPL-2.0

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};

pub const APP_ID: &str = "dev.korbeil.opencode-go-statusbar";
pub const ICON_NAME: &str = "dev.korbeil.opencode-go-statusbar-symbolic";

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub key: String,
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
