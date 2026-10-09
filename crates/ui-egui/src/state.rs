//! UI-only state, as plain serde data (automation reads and sets it; nothing here is behaviour).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tab {
    #[default]
    Apps,
    Settings,
}

impl Tab {
    pub fn label(self) -> &'static str {
        match self {
            Tab::Apps => "Apps",
            Tab::Settings => "Settings",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiState {
    pub tab: Tab,
    /// The app list's filter (matches name, id and tagline, case-insensitively).
    pub search: String,
}
