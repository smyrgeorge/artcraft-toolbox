//! Opening a Crafting App with its control channel, for an agent to drive its window while the
//! user watches (docs/control-protocol.md › Opening a Crafting App for agents). How each app takes
//! it is the catalog's `control`; whether it opens with it, and on which port, are its settings
//! (`agentControl`, `controlPort`). `app.launch` applies them.

use std::path::{Path, PathBuf};

use artcraft_toolbox_catalog::{App, ControlStyle};
use serde::Serialize;
use serde_json::{Value, json};

use crate::install_cmds::refused;
use crate::{Result, Session};

/// An app's control channel as it would open now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlInfo {
    /// The app opens with it (its `agentControl` setting).
    pub on: bool,
    pub style: ControlStyle,
    /// The port it listens on; `None` when the app picks its own (it writes it to [`Self::file`]).
    pub port: Option<u16>,
    /// The catalog's port, the one it has until the user picks another.
    pub default_port: Option<u16>,
    /// Where it keeps its token or connection details, for the styles that have one; `None`
    /// also when this session has no data folder.
    pub file: Option<PathBuf>,
}

impl ControlInfo {
    /// `127.0.0.1:<port>`, when the port is known before the app opens.
    pub fn address(&self) -> Option<String> {
        self.port.map(|p| format!("127.0.0.1:{p}"))
    }

    /// What an agent needs to connect, for `app.launch`'s reply.
    fn to_json(&self) -> Value {
        json!({"style": self.style, "port": self.port, "address": self.address(), "file": self.file})
    }
}

impl Session {
    /// `app`'s control channel, settings applied; `None` when it has none (or one this build
    /// can't open).
    pub fn control_info(&self, app: &str) -> Option<ControlInfo> {
        let entry = self.catalog().get(app)?;
        let control = entry.control()?;
        let port = control.takes_port().then(|| self.settings().control_port(app).or(control.port)).flatten();
        let file = entry.control_file_name().and_then(|name| Some(self.store()?.control_dir().join(name)));
        Some(ControlInfo { on: self.settings().agent_control(app), style: control.style, port, default_port: control.port, file })
    }
}

/// The channel `app` opens with, and its arguments: `want` (`app.launch`'s `control`) or else its
/// setting. `None`: it opens without one. Asked for explicitly, an app without one is an error.
pub(crate) fn opening(s: &Session, app: &App, want: Option<bool>) -> Result<Option<(ControlInfo, Vec<String>)>> {
    let Some(mut info) = s.control_info(&app.id) else {
        return match want {
            Some(true) => Err(refused(format!("{} has no control channel", app.name))),
            _ => Ok(None),
        };
    };
    info.on = want.unwrap_or(info.on);
    if !info.on {
        return Ok(None);
    }
    let file = match (&info.file, app.control_file_name()) {
        (Some(file), _) => Some(prepare(file)?),
        (None, Some(_)) => return Err(refused(format!("this session has no data folder for {}'s control token", app.name))),
        (None, None) => None,
    };
    // A port style always has one (the catalog checks it); the file style ignores it.
    let args = app.control().and_then(|c| c.args(info.port.unwrap_or_default(), file.as_deref()));
    let Some(args) = args else { return Err(refused(format!("{} can't be opened with its control channel", app.name))) };
    Ok(Some((info, args)))
}

/// Make the folder of `file` (only its owner may read it: it holds tokens) and return the file as
/// an argument.
fn prepare(file: &Path) -> Result<String> {
    if let Some(dir) = file.parent() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir).map_err(|e| refused(format!("couldn't create {} ({e})", dir.display())))?;
    }
    file.to_str().map(str::to_string).ok_or_else(|| refused(format!("{} isn't a path an app can be handed", file.display())))
}

/// Does the channel of the app, already running, answer: on its port, or for an app that picks
/// its own, on the one it wrote to its file?
pub(crate) fn answers(info: &ControlInfo) -> bool {
    let port = match info.style {
        ControlStyle::File => {
            let text = info.file.as_ref().and_then(|f| read_small(f)).unwrap_or_default();
            serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("port")?.as_u64()).and_then(|p| u16::try_from(p).ok())
        }
        _ => info.port,
    };
    port.is_some_and(artcraft_toolbox_install::port_answers)
}

/// The app's connection file, when it is small enough to be one (a few dozen bytes).
fn read_small(file: &Path) -> Option<String> {
    std::fs::metadata(file).ok().filter(|m| m.len() <= 4096)?;
    std::fs::read_to_string(file).ok()
}

/// `app.launch`'s `control` in its reply.
pub(crate) fn reply(opened: Option<&ControlInfo>) -> Value {
    opened.map_or(Value::Null, ControlInfo::to_json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::temp;
    use artcraft_toolbox_store::Store;

    fn session() -> (Session, PathBuf) {
        let dir = temp("control");
        let (s, _) = Session::open(crate::Catalog::builtin().unwrap(), Some(Store::open(&dir).unwrap()), None);
        (s, dir)
    }

    #[test]
    fn each_app_opens_its_own_way() {
        let (mut s, dir) = session();
        let control = dir.join("control");
        let photocraft = s.control_info("photocraft").unwrap();
        assert_eq!(
            photocraft,
            ControlInfo {
                on: false,
                style: ControlStyle::PortAndTokenFile,
                port: Some(7878),
                default_port: Some(7878),
                file: Some(control.join("photocraft-control.token")),
            }
        );
        assert_eq!(photocraft.address().as_deref(), Some("127.0.0.1:7878"));
        assert_eq!(s.control_info("pdfcraft").map(|c| (c.port, c.file)), Some((None, Some(control.join("pdfcraft-control.json")))));
        assert_eq!(s.control_info("artcraft"), None);
        // Off: no arguments, unless asked for.
        let app = |s: &Session, id: &str| s.catalog().get(id).unwrap().clone();
        assert_eq!(opening(&s, &app(&s, "vectorcraft"), None).unwrap(), None);
        let (_, args) = opening(&s, &app(&s, "vectorcraft"), Some(true)).unwrap().unwrap();
        assert_eq!(args, ["--control", "7879"]);
        assert!(opening(&s, &app(&s, "artcraft"), Some(true)).unwrap_err().to_string().contains("no control channel"));
        assert_eq!(opening(&s, &app(&s, "artcraft"), None).unwrap(), None);
        // The settings: on, on another port.
        s.execute("app.settings.set", json!({"app": "photocraft", "agentControl": true, "controlPort": 9100})).unwrap();
        let (info, args) = opening(&s, &app(&s, "photocraft"), None).unwrap().unwrap();
        let token = control.join("photocraft-control.token");
        assert_eq!(args, ["--control".to_string(), "9100".into(), "--control-token-file".into(), token.display().to_string()]);
        assert_eq!(info.default_port, Some(7878));
        assert!(control.is_dir(), "the app writes its token there but doesn't make the folder");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&control).unwrap().permissions().mode() & 0o777, 0o700);
        }
        assert_eq!(opening(&s, &app(&s, "photocraft"), Some(false)).unwrap(), None, "asked not to: it opens as usual");
        let (_, args) = opening(&s, &app(&s, "pdfcraft"), Some(true)).unwrap().unwrap();
        assert_eq!(args, ["--control".to_string(), control.join("pdfcraft-control.json").display().to_string()]);
    }

    /// `app.launch` hands the app its channel's arguments, from the setting or the `control` param,
    /// and says where an agent connects. The app is a script that writes down its arguments.
    #[cfg(unix)]
    #[test]
    fn launch_opens_the_app_with_its_channel() {
        use artcraft_toolbox_model::{Installation, Inventory, PackageKind};
        use std::os::unix::fs::PermissionsExt;
        let (mut s, dir) = session();
        let (exe, seen) = (dir.join("vectorcraft.AppImage"), dir.join("args.txt"));
        std::fs::write(&exe, format!("#!/bin/sh\necho \"$@\" > '{}.tmp' && mv '{0}.tmp' '{0}'\n", seen.display())).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut inv = Inventory::default();
        let path = exe.to_str().unwrap().to_string();
        let inst = Installation {
            app: "vectorcraft".into(),
            version: crate::Version::new(0, 7, 0),
            kind: PackageKind::AppImage,
            path,
            installed_at: 0,
            active: true,
            trust: None,
        };
        inv.record(inst).unwrap();
        s.set_inventory(inv);
        let launched = |s: &mut Session, p: Value| {
            let _ = std::fs::remove_file(&seen);
            let out = s.execute("app.launch", p).unwrap();
            for _ in 0..200 {
                if let Ok(args) = std::fs::read_to_string(&seen) {
                    return (out, args.trim().to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("the app didn't start");
        };
        let (out, args) = launched(&mut s, json!({"app": "vectorcraft"}));
        assert_eq!((out["control"].clone(), args.as_str()), (Value::Null, ""));
        s.execute("app.settings.set", json!({"app": "vectorcraft", "agentControl": true, "controlPort": 9200})).unwrap();
        let (out, args) = launched(&mut s, json!({"app": "vectorcraft"}));
        assert_eq!(args, "--control 9200");
        assert_eq!(out["control"], json!({"style": "port", "port": 9200, "address": "127.0.0.1:9200", "file": null}));
        let (_, args) = launched(&mut s, json!({"app": "vectorcraft", "control": false}));
        assert_eq!(args, "", "asked not to");
        for bad in [json!({"app": "vectorcraft", "control": "yes"}), json!({"app": "vectorcraft", "control": 1})] {
            assert!(s.execute("app.launch", bad.clone()).unwrap_err().to_string().contains("`control` must be true or false"), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_say_how_an_app_opens_for_agents() {
        let (mut s, dir) = session();
        let got = s.execute("app.settings.get", json!({"app": "photocraft"})).unwrap();
        assert_eq!((got["agentControl"].clone(), got["controlPort"].clone()), (json!(false), json!(7878)));
        assert_eq!(got["control"]["file"], json!(dir.join("control").join("photocraft-control.token")));
        let got = s.execute("app.settings.set", json!({"app": "photocraft", "agentControl": true, "controlPort": 9300})).unwrap();
        assert_eq!((got["agentControl"].clone(), got["controlPort"].clone(), got["control"]["defaultPort"].clone()), (json!(true), json!(9300), json!(7878)));
        assert_eq!(got["overrides"], json!({"agentControl": true, "controlPort": 9300}));
        assert_eq!(s.execute("app.settings.get", json!({"app": "artcraft"})).unwrap()["control"], Value::Null);
        for (p, want) in [
            (json!({"app": "artcraft", "agentControl": true}), "no control channel"),
            (json!({"app": "artcraft", "controlPort": 9000}), "no control channel"),
            (json!({"app": "pdfcraft", "controlPort": 9000}), "picks its own control port"),
            (json!({"app": "photocraft", "controlPort": 80}), "controlPort must be"),
            (json!({"app": "photocraft", "agentControl": "on"}), "agentControl must be"),
        ] {
            let e = s.execute("app.settings.set", p.clone()).unwrap_err().to_string();
            assert!(e.contains(want), "{p}: {e}");
        }
        // Clearing is always fine, even for an app without one.
        s.execute("app.settings.set", json!({"app": "artcraft", "agentControl": null})).unwrap();
        s.execute("app.settings.set", json!({"app": "pdfcraft", "agentControl": true})).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_token_needs_a_data_folder() {
        let s = Session::new().unwrap();
        let photocraft = s.catalog().get("photocraft").unwrap().clone();
        assert!(opening(&s, &photocraft, Some(true)).unwrap_err().to_string().contains("no data folder"));
        let wordcraft = s.catalog().get("wordcraft").unwrap().clone();
        assert!(opening(&s, &wordcraft, Some(true)).unwrap().is_some(), "a port is enough");
    }

    #[test]
    fn a_channel_answers_on_its_port_or_the_one_in_its_file() {
        let (s, dir) = session();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let open = listener.local_addr().unwrap().port();
        let mut info = s.control_info("vectorcraft").unwrap();
        info.port = Some(open);
        assert!(answers(&info));
        let mut pdf = s.control_info("pdfcraft").unwrap();
        assert!(!answers(&pdf), "no file yet");
        let file = dir.join("pdfcraft-control.json");
        std::fs::write(&file, json!({"port": open, "token": "x", "pid": 1}).to_string()).unwrap();
        pdf.file = Some(file.clone());
        assert!(answers(&pdf));
        for junk in ["not json".to_string(), json!({"port": 70000}).to_string(), "x".repeat(5000)] {
            std::fs::write(&file, junk).unwrap();
            assert!(!answers(&pdf));
        }
    }
}
