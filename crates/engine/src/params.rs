//! Reading command params. Params come from agents and scripts: every accessor validates and
//! returns `BadParams`, never panics.

use artcraft_toolbox_catalog::App;
use serde_json::{Map, Value};

use crate::{EngineError, Result, Session, echo};

pub(crate) fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// The params object (execute guarantees one).
pub(crate) fn object(p: &Value) -> Result<&Map<String, Value>> {
    p.as_object().ok_or_else(|| EngineError::BadParams(format!("params must be a JSON object, got {}", kind(p))))
}

/// Refuse keys the command doesn't take: a typo (`"ap"`) must not silently mean "no value".
pub(crate) fn only(p: &Value, allowed: &[&str]) -> Result<()> {
    match object(p)?.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(EngineError::BadParams(format!(
            "unknown param `{}` (takes: {})",
            echo(k),
            if allowed.is_empty() { "none".to_string() } else { allowed.join(", ") }
        ))),
        None => Ok(()),
    }
}

pub(crate) fn str<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    match object(p)?.get(key) {
        Some(Value::String(s)) => Ok(s),
        Some(other) => Err(EngineError::BadParams(format!("`{key}` must be a string, got {}", kind(other)))),
        None => Err(EngineError::BadParams(format!("missing `{key}`"))),
    }
}

/// The optional `version` param.
pub(crate) fn version(p: &Value) -> Result<Option<artcraft_toolbox_release::Version>> {
    match object(p)?.get("version") {
        None => Ok(None),
        Some(Value::String(v)) => artcraft_toolbox_release::Version::parse(v).map(Some).map_err(|e| EngineError::BadParams(e.to_string())),
        Some(other) => Err(EngineError::BadParams(format!("`version` must be a string, got {}", kind(other)))),
    }
}

/// The catalog app named by the `app` param.
pub(crate) fn app<'a>(s: &'a Session, p: &Value) -> Result<&'a App> {
    let id = str(p, "app")?;
    s.catalog().get(id).ok_or_else(|| EngineError::UnknownApp(echo(id)))
}

/// The catalog app named by `app`, or the toolbox itself: everything with a release feed.
pub(crate) fn feed_app<'a>(s: &'a Session, p: &Value) -> Result<&'a App> {
    let id = str(p, "app")?;
    s.feed_app(id).ok_or_else(|| EngineError::UnknownApp(echo(id)))
}
