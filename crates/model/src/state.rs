//! What the toolbox remembers between runs that isn't a setting: which update versions the user
//! has already been told about, so a restart doesn't notify again.

use std::collections::BTreeMap;

use artcraft_toolbox_release::{PackageKind, Version};
use serde::{Deserialize, Serialize};

use crate::{Error, Result, Trust};

/// Most entries kept (one per app).
pub const MAX_NOTIFIED: usize = 256;
/// Longest staged path kept.
pub const MAX_PATH: usize = 4096;

/// A newer version of the toolbox itself, downloaded, verified and placed beside the running
/// one, to be swapped in at the next start (`toolbox.update`; docs/architecture.md § 7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedUpdate {
    pub version: Version,
    pub kind: PackageKind,
    /// The staged `.app` bundle, `.exe` or AppImage.
    pub path: String,
    /// Unix seconds.
    #[serde(default)]
    pub staged_at: u64,
    /// Its platform signature when it was staged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust: Option<Trust>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolboxState {
    /// Per app (the toolbox itself included): the newest version an update notification was
    /// shown for.
    pub notified: BTreeMap<String, Version>,
    /// The toolbox's own update waiting for the next start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub staged: Option<StagedUpdate>,
}

impl ToolboxState {
    /// Load `state.json`; a file that doesn't parse is an error (the caller starts empty). A
    /// staged update whose path is unusable is dropped (the files, if any, are found again by
    /// their version).
    pub fn from_json(text: &str) -> Result<ToolboxState> {
        let mut s: ToolboxState = crate::from_json("state", text)?;
        while s.notified.len() > MAX_NOTIFIED {
            s.notified.pop_last();
        }
        if s.staged.as_ref().is_some_and(|u| u.path.is_empty() || u.path.len() > MAX_PATH || u.path.contains('\0')) {
            s.staged = None;
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

    #[test]
    fn a_staged_update_round_trips_and_a_hostile_one_is_dropped() {
        let staged = StagedUpdate {
            version: Version::new(0, 2, 0),
            kind: PackageKind::AppImage,
            path: "/data/self-update/x".into(),
            staged_at: 7,
            trust: Some(Trust::Unsigned),
        };
        let s = ToolboxState { notified: BTreeMap::new(), staged: Some(staged.clone()) };
        let text = s.to_json().unwrap();
        assert!(text.contains("\"staged\"") && text.contains("\"app_image\""), "{text}");
        assert_eq!(ToolboxState::from_json(&text).unwrap(), s);
        assert!(!ToolboxState::default().to_json().unwrap().contains("staged"), "absent when there is none");
        for bad in ["", "a\u{0}b", &"p".repeat(MAX_PATH + 1)] {
            let t = serde_json::json!({"staged": {"version": "0.2.0", "kind": "app_image", "path": bad}}).to_string();
            assert_eq!(ToolboxState::from_json(&t).unwrap().staged, None, "{bad:?}");
        }
        assert!(ToolboxState::from_json(r#"{"staged":{"version":"nope","kind":"app_image","path":"/x"}}"#).is_err());
        assert!(ToolboxState::from_json(r#"{"staged":{"version":"0.2.0","kind":"floppy","path":"/x"}}"#).is_err());
    }
}
