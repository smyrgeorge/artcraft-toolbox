//! The command registry. Ids are `<area>.<verb>` (`app.status`, `settings.set`); each area lives
//! in its own `<area>_cmds.rs` module with a `specs()` function, registered below.

use std::sync::OnceLock;

use serde_json::Value;

use crate::{JobId, Result, Session};

type Run = fn(&mut Session, &Value) -> Result<Value>;
type Enabled = fn(&Session) -> std::result::Result<(), String>;
type Start = fn(&mut Session, &Value) -> Result<JobId>;

/// Metadata + implementation for one command.
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Human/agent-readable params, e.g. `{"app":"<id>"}`. What `commands` lists for agents.
    pub params: &'static str,
    /// Precondition (greys the action out in the UI).
    pub enabled: Enabled,
    /// Runs the command to completion (what [`Session::execute`] calls).
    pub run: Run,
    /// For long commands: starts it as a background job instead (what [`Session::start`] calls).
    pub start: Option<Start>,
}

pub(crate) fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// Every registered command, in registration order.
pub fn command_specs() -> &'static [CommandSpec] {
    static SPECS: OnceLock<Vec<CommandSpec>> = OnceLock::new();
    SPECS.get_or_init(|| {
        let mut v = Vec::new();
        v.extend(crate::catalog_cmds::specs());
        v.extend(crate::status_cmds::specs());
        v.extend(crate::settings_cmds::specs());
        v.extend(crate::update_cmds::specs());
        v
    })
}

pub fn find(id: &str) -> Option<&'static CommandSpec> {
    command_specs().iter().find(|s| s.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_well_formed() {
        let specs = command_specs();
        for (i, s) in specs.iter().enumerate() {
            assert!(specs.iter().skip(i + 1).all(|o| o.id != s.id), "duplicate id {}", s.id);
            assert!(s.id.contains('.') && s.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.'), "{}", s.id);
            assert!(!s.label.is_empty() && s.params.starts_with('{'), "{}", s.id);
        }
    }
}
