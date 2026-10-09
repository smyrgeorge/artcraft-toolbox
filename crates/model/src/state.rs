//! What the toolbox remembers between runs that isn't a setting: which update versions the user
//! has already been told about, so a restart doesn't notify again.

use std::collections::BTreeMap;

use artcraft_toolbox_release::Version;
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Most entries kept (one per app).
pub const MAX_NOTIFIED: usize = 256;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolboxState {
    /// Per app: the newest version an update notification was shown for.
    pub notified: BTreeMap<String, Version>,
}

impl ToolboxState {
    /// Load `state.json`; a file that doesn't parse is an error (the caller starts empty).
    pub fn from_json(text: &str) -> Result<ToolboxState> {
        let mut s: ToolboxState = crate::from_json("state", text)?;
        while s.notified.len() > MAX_NOTIFIED {
            s.notified.pop_last();
        }
        Ok(s)
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| Error::Invalid(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_bounds() {
        let mut s = ToolboxState::default();
        s.notified.insert("photocraft".into(), Version::new(0, 5, 0));
        assert_eq!(ToolboxState::from_json(&s.to_json().unwrap()).unwrap(), s);
        assert_eq!(ToolboxState::from_json("{}").unwrap(), ToolboxState::default());
        assert!(ToolboxState::from_json(r#"{"notified":{"x":"nope"}}"#).is_err());
        let many: serde_json::Map<String, serde_json::Value> = (0..MAX_NOTIFIED + 3).map(|i| (format!("a{i}"), serde_json::json!("1.0.0"))).collect();
        assert_eq!(ToolboxState::from_json(&serde_json::json!({ "notified": many }).to_string()).unwrap().notified.len(), MAX_NOTIFIED);
    }
}
