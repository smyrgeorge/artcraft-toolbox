//! Dependency layering rules (docs/architecture.md › Layers).
//!
//! The rule engine works on a small, metadata-independent model so it can be unit-tested;
//! `from_metadata` builds that model from `cargo metadata`.

use serde_json::Value;

/// Where a workspace crate sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Regular layered crate.
    Layer(u8),
    /// Layer 0, and additionally may depend on no workspace crate at all.
    Standalone,
    /// Test tooling: may depend on anything up to L5 (it sits at L6 for rule purposes); other
    /// crates may use it only as a dev-dependency.
    Testkit,
    /// Binaries and build tooling: exempt from the rules.
    Exempt,
}

impl Class {
    fn layer(self) -> Option<u8> {
        match self {
            Class::Layer(l) => Some(l),
            Class::Standalone => Some(0),
            Class::Testkit => Some(6),
            Class::Exempt => None,
        }
    }
}

/// The layering table. Names are package names without the `artcraft-toolbox-` prefix. Crates
/// marked "planned" don't exist yet; their layer is decided here so the first commit that adds
/// one lands in the right place (docs/architecture.md › Crate map).
pub const TABLE: &[(&str, Class)] = &[
    // L0: the release contract and the catalog, pure data.
    ("release", Class::Standalone),
    ("catalog", Class::Standalone),
    // L1: toolbox state (inventory, settings).
    ("model", Class::Layer(1)),
    // L2: release feeds and update status.
    ("feed", Class::Layer(2)),
    // L3: the edges that touch the machine: HTTPS (net), the toolbox's own files (store), per-OS
    // install/launch (install, planned).
    ("net", Class::Layer(3)),
    ("store", Class::Layer(3)),
    ("install", Class::Layer(3)),
    // L4: jobs that combine them (planned): download → verify → install → record, self-update.
    ("jobs", Class::Layer(4)),
    // L5: Session + command registry.
    ("engine", Class::Layer(5)),
    // L6: frontends' libraries.
    ("ui-egui", Class::Layer(6)),
    ("automation", Class::Layer(6)),
    ("testkit", Class::Testkit),
    // L7 apps and tooling.
    ("artcraft-toolbox", Class::Exempt),
    ("cli", Class::Exempt),
    ("xtask", Class::Exempt),
];

/// External crates that constitute a UI toolkit / windowing dependency. Entries ending in `*`
/// are prefixes.
pub const UI_CRATES: &[&str] = &["egui", "eframe", "winit", "egui_kittest", "egui_extras", "rfd", "muda", "tray-icon"];

/// First layer allowed to use UI crates.
pub const UI_MIN_LAYER: u8 = 6;

/// External crates that do network, async or archive I/O. The pure core (L0–L2) must stay
/// testable from bytes alone, so these start at L3.
pub const IO_CRATES: &[&str] =
    &["ureq", "reqwest", "hyper*", "curl", "tokio*", "async-*", "rustls*", "native-tls", "zip", "tar", "flate2", "xz2", "sevenz-rust"];

/// First layer allowed to use I/O crates.
pub const IO_MIN_LAYER: u8 = 3;

pub fn short_name(pkg: &str) -> &str {
    pkg.strip_prefix("artcraft-toolbox-").unwrap_or(pkg)
}

pub fn classify(pkg: &str) -> Option<Class> {
    let s = short_name(pkg);
    TABLE.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
}

fn listed(list: &[&str], name: &str) -> bool {
    list.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *p,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub kind: DepKind,
    /// `true` if the dependency is a workspace member.
    pub workspace: bool,
}

#[derive(Debug, Clone)]
pub struct Crate {
    pub name: String,
    pub deps: Vec<Dep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    Unregistered { krate: String },
    Upward { krate: String, dep: String, from: u8, to: u8, kind: DepKind },
    StandaloneHasWorkspaceDep { krate: String, dep: String },
    TestkitAsNormalDep { krate: String },
    UiBelowL6 { krate: String, dep: String, layer: u8 },
    IoBelowL3 { krate: String, dep: String, layer: u8 },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::Unregistered { krate } => {
                write!(f, "{krate}: unknown workspace crate; register it in xtask/src/layers.rs TABLE (see docs/architecture.md › Layers)")
            }
            Violation::Upward { krate, dep, from, to, kind } => {
                write!(f, "{krate} (L{from}) -> {dep} (L{to}) [{kind:?}]: may only depend on strictly lower layers")
            }
            Violation::StandaloneHasWorkspaceDep { krate, dep } => {
                write!(f, "{krate}: standalone crate must not depend on workspace crate {dep}")
            }
            Violation::TestkitAsNormalDep { krate } => {
                write!(f, "{krate}: artcraft-toolbox-testkit may only be a dev-dependency")
            }
            Violation::UiBelowL6 { krate, dep, layer } => {
                write!(f, "{krate} (L{layer}) depends on UI crate `{dep}`; UI toolkits are only allowed in L6+")
            }
            Violation::IoBelowL3 { krate, dep, layer } => {
                write!(f, "{krate} (L{layer}) depends on I/O crate `{dep}`; network, async and archive crates start at L3 (the core is pure)")
            }
        }
    }
}

/// Check all rules. Returns violations sorted for stable output.
pub fn check(crates: &[Crate]) -> Vec<Violation> {
    let mut out = Vec::new();
    for c in crates {
        let Some(class) = classify(&c.name) else {
            out.push(Violation::Unregistered { krate: c.name.clone() });
            continue;
        };
        if class == Class::Exempt {
            continue;
        }
        let layer = class.layer().unwrap_or(0);
        for d in &c.deps {
            // Self dev-dependencies (e.g. to enable features in tests) are fine.
            if d.name == c.name {
                continue;
            }
            if d.workspace {
                if class == Class::Standalone {
                    out.push(Violation::StandaloneHasWorkspaceDep { krate: c.name.clone(), dep: d.name.clone() });
                    continue;
                }
                match classify(&d.name) {
                    // Unregistered deps are reported on their own entry.
                    None => {}
                    Some(Class::Testkit) => {
                        if d.kind != DepKind::Dev {
                            out.push(Violation::TestkitAsNormalDep { krate: c.name.clone() });
                        }
                    }
                    Some(dc) => {
                        let to = dc.layer().unwrap_or(u8::MAX);
                        if to >= layer {
                            out.push(Violation::Upward {
                                krate: c.name.clone(),
                                dep: d.name.clone(),
                                from: layer,
                                to: if to == u8::MAX { 7 } else { to },
                                kind: d.kind,
                            });
                        }
                    }
                }
            } else if d.kind != DepKind::Dev {
                // Dev-dependencies may use anything: tests drive the UI (egui_kittest) or fake I/O.
                if layer < UI_MIN_LAYER && listed(UI_CRATES, &d.name) {
                    out.push(Violation::UiBelowL6 { krate: c.name.clone(), dep: d.name.clone(), layer });
                }
                if layer < IO_MIN_LAYER && listed(IO_CRATES, &d.name) {
                    out.push(Violation::IoBelowL3 { krate: c.name.clone(), dep: d.name.clone(), layer });
                }
            }
        }
    }
    out.sort_by_key(|v| v.to_string());
    out.dedup();
    out
}

/// Build the model from `cargo metadata --format-version 1 --no-deps`.
pub fn from_metadata(meta: &Value) -> Result<Vec<Crate>, String> {
    let pkgs = meta["packages"].as_array().ok_or("metadata: no packages array")?;
    let members: Vec<&str> = pkgs.iter().filter_map(|p| p["name"].as_str()).collect();
    let mut out = Vec::new();
    for p in pkgs {
        let name = p["name"].as_str().ok_or("package without name")?.to_owned();
        let mut deps = Vec::new();
        for d in p["dependencies"].as_array().into_iter().flatten() {
            let dname = d["name"].as_str().unwrap_or_default().to_owned();
            let kind = match d["kind"].as_str() {
                Some("dev") => DepKind::Dev,
                Some("build") => DepKind::Build,
                _ => DepKind::Normal,
            };
            let workspace = members.contains(&dname.as_str()) || d["path"].is_string();
            deps.push(Dep { name: dname, kind, workspace });
        }
        out.push(Crate { name, deps });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn describe(class: Option<Class>) -> String {
    match class {
        Some(Class::Layer(l)) => format!("L{l}"),
        Some(Class::Standalone) => "L0 standalone".into(),
        Some(Class::Testkit) => "testkit".into(),
        Some(Class::Exempt) => "exempt".into(),
        None => "UNREGISTERED".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, deps: &[(&str, DepKind, bool)]) -> Crate {
        Crate { name: name.into(), deps: deps.iter().map(|(n, k, w)| Dep { name: (*n).into(), kind: *k, workspace: *w }).collect() }
    }
    use DepKind::*;

    #[test]
    fn clean_downward_graph_passes() {
        let g = [
            c("artcraft-toolbox-release", &[("serde", Normal, false)]),
            c("artcraft-toolbox-model", &[("artcraft-toolbox-release", Normal, true)]),
            c("artcraft-toolbox-feed", &[("artcraft-toolbox-model", Normal, true), ("artcraft-toolbox-release", Normal, true)]),
            c("artcraft-toolbox-net", &[("ureq", Normal, false)]),
            c("artcraft-toolbox-engine", &[("artcraft-toolbox-feed", Normal, true), ("artcraft-toolbox-testkit", Dev, true)]),
            c("artcraft-toolbox-ui-egui", &[("artcraft-toolbox-engine", Normal, true), ("egui", Normal, false)]),
            c("artcraft-toolbox-cli", &[("artcraft-toolbox-ui-egui", Normal, true)]),
        ];
        assert!(check(&g).is_empty(), "{:?}", check(&g));
    }

    #[test]
    fn upward_and_sideways_dependencies_flagged() {
        let v = check(&[c("artcraft-toolbox-model", &[("artcraft-toolbox-engine", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 1, to: 5, .. }]));
        let v = check(&[c("artcraft-toolbox-net", &[("artcraft-toolbox-install", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 3, to: 3, .. }]));
        let v = check(&[c("artcraft-toolbox-model", &[("artcraft-toolbox-feed", Dev, true)])]);
        assert!(matches!(v[..], [Violation::Upward { kind: Dev, .. }]));
    }

    #[test]
    fn standalone_crates_have_no_workspace_deps() {
        for s in ["artcraft-toolbox-release", "artcraft-toolbox-catalog"] {
            let v = check(&[c(s, &[("artcraft-toolbox-model", Normal, true)])]);
            assert!(matches!(v[..], [Violation::StandaloneHasWorkspaceDep { .. }]), "{s}");
        }
        let v = check(&[c("artcraft-toolbox-catalog", &[("artcraft-toolbox-release", Normal, true)])]);
        assert!(matches!(v[..], [Violation::StandaloneHasWorkspaceDep { .. }]));
        assert!(check(&[c("artcraft-toolbox-feed", &[("artcraft-toolbox-release", Normal, true)])]).is_empty());
    }

    #[test]
    fn ui_crates_below_l6_flagged() {
        for dep in ["egui", "eframe", "winit", "rfd"] {
            let v = check(&[c("artcraft-toolbox-engine", &[(dep, Normal, false)])]);
            assert!(matches!(v[..], [Violation::UiBelowL6 { layer: 5, .. }]), "{dep}");
        }
        assert!(check(&[c("artcraft-toolbox-ui-egui", &[("egui", Normal, false)])]).is_empty());
        // Tests may drive a UI from below L6 (none do today, but it isn't a layering problem).
        assert!(check(&[c("artcraft-toolbox-engine", &[("egui_kittest", Dev, false)])]).is_empty());
    }

    #[test]
    fn io_crates_below_l3_flagged() {
        for dep in ["ureq", "reqwest", "hyper-util", "tokio", "zip", "rustls-pki-types"] {
            let v = check(&[c("artcraft-toolbox-feed", &[(dep, Normal, false)])]);
            assert!(matches!(v[..], [Violation::IoBelowL3 { layer: 2, .. }]), "{dep}");
        }
        assert!(check(&[c("artcraft-toolbox-net", &[("ureq", Normal, false)])]).is_empty());
        assert!(check(&[c("artcraft-toolbox-feed", &[("serde_json", Normal, false)])]).is_empty());
    }

    #[test]
    fn unregistered_crate_is_error() {
        let v = check(&[c("artcraft-toolbox-mystery", &[])]);
        assert!(matches!(&v[..], [Violation::Unregistered { krate }] if krate == "artcraft-toolbox-mystery"));
        assert!(v[0].to_string().contains("register"));
    }

    #[test]
    fn testkit_only_as_dev_dependency() {
        let v = check(&[c("artcraft-toolbox-feed", &[("artcraft-toolbox-testkit", Normal, true)])]);
        assert!(matches!(v[..], [Violation::TestkitAsNormalDep { .. }]));
        assert!(check(&[c("artcraft-toolbox-feed", &[("artcraft-toolbox-testkit", Dev, true)])]).is_empty());
        assert!(check(&[c("artcraft-toolbox-testkit", &[("artcraft-toolbox-engine", Normal, true)])]).is_empty());
        assert!(!check(&[c("artcraft-toolbox-testkit", &[("artcraft-toolbox-ui-egui", Normal, true)])]).is_empty());
    }

    #[test]
    fn apps_and_xtask_exempt() {
        for app in ["artcraft-toolbox", "artcraft-toolbox-cli", "xtask"] {
            assert!(check(&[c(app, &[("egui", Normal, false), ("artcraft-toolbox-ui-egui", Normal, true)])]).is_empty());
        }
    }

    #[test]
    fn the_real_workspace_is_registered() {
        let crates = from_metadata(&crate::metadata().unwrap()).unwrap();
        assert!(crates.len() >= 9);
        assert!(check(&crates).is_empty(), "{:?}", check(&crates));
    }

    #[test]
    fn metadata_parsing() {
        let meta: Value = serde_json::from_str(
            r#"{"packages":[
                {"name":"artcraft-toolbox-model","dependencies":[
                    {"name":"artcraft-toolbox-release","kind":null,"path":"/x/crates/release"},
                    {"name":"serde","kind":null},
                    {"name":"proptest","kind":"dev"}]},
                {"name":"artcraft-toolbox-release","dependencies":[]}
            ]}"#,
        )
        .unwrap();
        let g = from_metadata(&meta).unwrap();
        assert_eq!(g.len(), 2);
        let model = g.iter().find(|c| c.name == "artcraft-toolbox-model").unwrap();
        assert!(model.deps[0].workspace && !model.deps[1].workspace);
        assert_eq!(model.deps[2].kind, Dev);
        assert!(check(&g).is_empty());
    }
}
