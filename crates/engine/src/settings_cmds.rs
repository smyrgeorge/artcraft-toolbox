//! `settings.get` and `settings.set`.

use serde_json::Value;

use crate::commands::{CommandSpec, always};
use crate::{EngineError, Result, Session, params};

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec { id: "settings.get", label: "Settings", params: "{}", enabled: always, run: get },
        CommandSpec {
            id: "settings.set",
            label: "Change Settings",
            params: r#"{"channel"?:"stable|prerelease","checkIntervalHours"?:0..168,"autoUpdate"?:bool,"keepPrevious"?:0..5,"installDir"?:"<path>"|null}"#,
            enabled: always,
            run: set,
        },
    ]
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &[])?;
    serde_json::to_value(s.settings()).map_err(|e| EngineError::BadParams(e.to_string()))
}

/// All or nothing: one bad value leaves every setting unchanged.
fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let changes = params::object(p)?;
    if changes.is_empty() {
        return Err(EngineError::BadParams("no settings given".into()));
    }
    let mut next = s.settings().clone();
    for (k, v) in changes {
        next.set(k, v)?;
    }
    s.set_settings(next);
    get(s, &Value::Object(Default::default()))
}

#[cfg(test)]
mod tests {
    use crate::Session;
    use serde_json::json;

    #[test]
    fn set_is_all_or_nothing() {
        let mut s = Session::new().unwrap();
        let out = s.execute("settings.set", json!({"channel": "prerelease", "keepPrevious": 2})).unwrap();
        assert_eq!((out["channel"].as_str(), out["keepPrevious"].as_u64()), (Some("prerelease"), Some(2)));
        let before = s.execute("settings.get", json!({})).unwrap();
        assert!(s.execute("settings.set", json!({"channel": "stable", "keepPrevious": 99})).is_err());
        assert_eq!(s.execute("settings.get", json!({})).unwrap(), before);
        assert!(s.execute("settings.set", json!({})).is_err());
    }
}
