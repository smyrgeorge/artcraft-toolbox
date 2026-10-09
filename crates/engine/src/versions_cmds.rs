//! An app's versions: `app.versions`, `app.rollback` (switch to a kept version), `app.adopt`
//! (take over a copy installed by hand), `apps.updateAll` and `apps.rescan`.
//!
//! Rollback activates a version the toolbox kept (`keepPrevious`), with no download: on macOS the
//! bundles swap places, elsewhere the shortcut or desktop entry is pointed at it. Its signature was
//! checked when it was installed.

use std::path::{Path, PathBuf};

use artcraft_toolbox_install::Current;
use artcraft_toolbox_model::Installation;
use artcraft_toolbox_release::Version;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::install_cmds::{busy, can_change_installs, info, kind_for, refused, start_update_of};
use crate::{EngineError, Result, Session, Status, params};

pub const VERSIONS: &str = "app.versions";
pub const ROLLBACK: &str = "app.rollback";
pub const ADOPT: &str = "app.adopt";
pub const UPDATE_ALL: &str = "apps.updateAll";
pub const RESCAN: &str = "apps.rescan";

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec { id: VERSIONS, label: "Installed versions", params: r#"{"app":"<id>"}"#, enabled: always, run: versions, start: None },
        CommandSpec {
            id: ROLLBACK,
            label: "Switch version",
            params: r#"{"app":"<id>","version"?:"x.y.z"}"#,
            enabled: can_change_installs,
            run: rollback,
            start: None,
        },
        CommandSpec { id: ADOPT, label: "Adopt", params: r#"{"app":"<id>"}"#, enabled: can_change_installs, run: adopt, start: None },
        CommandSpec { id: UPDATE_ALL, label: "Update all", params: r#"{"onlyAutomatic"?:bool}"#, enabled: can_update, run: update_all, start: None },
        CommandSpec { id: RESCAN, label: "Look for installed apps", params: "{}", enabled: always, run: rescan, start: None },
    ]
}

fn can_update(s: &Session) -> std::result::Result<(), String> {
    if !s.online() {
        return Err("this session has no network access".into());
    }
    can_change_installs(s)
}

fn versions(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &["app"])?;
    let app = params::app(s, p)?;
    let installed: Vec<Value> = s
        .inventory()
        .versions(&app.id)
        .into_iter()
        .map(|i| json!({"version": i.version, "active": i.active, "path": i.path, "kind": i.kind, "installedAt": i.installed_at, "trust": i.trust}))
        .collect();
    let found: Vec<Value> = s.found(&app.id).iter().map(|(v, path)| json!({"version": v, "path": path})).collect();
    Ok(json!({"app": app.id, "installed": installed, "foundOutsideToolbox": found}))
}

/// Switch to a kept version: `version`, or the newest kept one older than the active one.
fn rollback(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &["app", "version"])?;
    let app = params::app(s, p)?.clone();
    let wanted = params::version(p)?;
    switch(s, &app, wanted)
}

pub(crate) fn switch(s: &mut Session, app: &artcraft_toolbox_catalog::App, wanted: Option<Version>) -> Result<Value> {
    busy(s, app)?;
    let current = s.inventory().current(&app.id).filter(|i| i.active).cloned().ok_or_else(|| refused(format!("{} isn't installed", app.name)))?;
    let target: Installation = match &wanted {
        Some(v) if *v == current.version => return Err(refused(format!("{} {v} is already the version in use", app.name))),
        Some(v) => s
            .inventory()
            .get(&app.id, v)
            .cloned()
            .ok_or_else(|| refused(format!("{} {v} isn't kept on this computer: update to it instead (app.update)", app.name)))?,
        None => s
            .inventory()
            .versions(&app.id)
            .into_iter()
            .find(|i| !i.active && i.version < current.version)
            .cloned()
            .ok_or_else(|| refused(format!("no earlier version of {} is kept on this computer", app.name)))?,
    };
    let layout = s.layout.clone().ok_or_else(|| refused("this session has no install location"))?;
    let (bundle_id, to, from) = (app.bundle_id(), target.version.to_string(), current.version.to_string());
    let was = Current { path: Path::new(&current.path), version: &from };
    let activated = artcraft_toolbox_install::activate(&layout, &info(app, &bundle_id, &to, None), target.kind, Path::new(&target.path), Some(was))?;
    let utf8 = |p: &Path| {
        p.to_str()
            .map(str::to_string)
            .ok_or_else(|| EngineError::Install(artcraft_toolbox_install::Error::Io("the install location is not a UTF-8 path".into())))
    };
    s.inventory.activate(&app.id, &target.version)?;
    s.inventory.set_path(&app.id, &target.version, utf8(&activated.active)?)?;
    if let Some(prev) = &activated.previous {
        s.inventory.set_path(&app.id, &current.version, utf8(prev)?)?;
    }
    s.save_inventory()?;
    Ok(json!({"app": app.id, "version": target.version, "previous": current.version, "path": activated.active}))
}

/// Take over the copies of an app found installed outside the toolbox (`apps.rescan`): record
/// them, the newest active, after checking their signatures. A broken signature is not adopted.
fn adopt(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &["app"])?;
    let app = params::app(s, p)?.clone();
    busy(s, &app)?;
    if !s.inventory().versions(&app.id).is_empty() {
        return Err(refused(format!("{} is already managed by the toolbox", app.name)));
    }
    s.rescan();
    let found: Vec<(Version, PathBuf)> = s.found(&app.id).to_vec();
    if found.is_empty() {
        return Err(refused(format!("no copy of {} installed outside the toolbox was found", app.name)));
    }
    let kind = s.host().and_then(|h| kind_for(h.os)).ok_or_else(|| refused("no Crafting App is built for this computer"))?;
    let (mut adopted, mut warnings) = (Vec::new(), Vec::new());
    // Oldest first: recording makes each the active one, so the newest ends up active.
    for (version, path) in found.iter().rev() {
        let Some(text) = path.to_str() else {
            warnings.push(format!("{} {version}: its path is not UTF-8", app.name));
            continue;
        };
        match artcraft_toolbox_install::verify_signature(kind, path) {
            Ok(trust) => {
                let inst = Installation {
                    app: app.id.clone(),
                    version: version.clone(),
                    kind,
                    path: text.to_string(),
                    installed_at: s.now(),
                    active: true,
                    trust: Some(trust),
                };
                s.inventory.record(inst)?;
                adopted.push(version.clone());
            }
            Err(e) => warnings.push(format!("{} {version} was not adopted: {e}", app.name)),
        }
    }
    if adopted.is_empty() {
        return Err(refused(warnings.join("; ")));
    }
    s.save_inventory()?;
    s.rescan();
    Ok(json!({"app": app.id, "adopted": adopted, "active": adopted.last(), "warnings": warnings}))
}

/// Start an update for every app that has one (`onlyAutomatic`: the apps set to update
/// automatically). Returns the jobs started and why the others weren't.
fn update_all(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &["onlyAutomatic"])?;
    let only_automatic = match params::object(p)?.get("onlyAutomatic") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(other) => return Err(EngineError::BadParams(format!("`onlyAutomatic` must be true or false, got {}", params::kind(other)))),
    };
    let (mut started, mut switched, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    for st in s.statuses() {
        if !matches!(st.status, Status::UpdateAvailable { .. }) || (only_automatic && !s.settings().auto_update_for(&st.id)) {
            continue;
        }
        let Some(app) = s.catalog().get(&st.id).cloned() else { continue };
        // A version kept on this computer is switched to, not downloaded again.
        if let Status::UpdateAvailable { latest, .. } = &st.status
            && s.inventory().get(&app.id, latest).is_some()
        {
            match switch(s, &app, Some(latest.clone())) {
                Ok(_) => switched.push(json!({"app": st.id, "version": latest})),
                Err(e) => skipped.push(json!({"app": st.id, "reason": e.to_string()})),
            }
            continue;
        }
        match start_update_of(s, &app, None) {
            Ok(job) => started.push(json!({"app": st.id, "job": job})),
            Err(e) => skipped.push(json!({"app": st.id, "reason": e.to_string()})),
        }
    }
    Ok(json!({"started": started, "switched": switched, "skipped": skipped}))
}

fn rescan(s: &mut Session, p: &Value) -> Result<Value> {
    params::only(p, &[])?;
    let changes = s.rescan();
    let found: serde_json::Map<String, Value> = s
        .catalog()
        .apps
        .iter()
        .filter(|a| !s.found(&a.id).is_empty())
        .map(|a| (a.id.clone(), json!(s.found(&a.id).iter().map(|(v, _)| v).collect::<Vec<_>>())))
        .collect();
    Ok(json!({"changes": changes, "foundOutsideToolbox": found}))
}
