//! Opening the session the apps use: the data directory (settings, inventory, feed cache) and
//! the GitHub client. The desktop app and the CLI open it the same way.

use std::path::PathBuf;
use std::sync::Arc;

use artcraft_toolbox_catalog::Catalog;
use artcraft_toolbox_net::{Client, Policy, Transport};
use artcraft_toolbox_store::Store;

use crate::{Result, Session, build_info};

/// A GitHub token the user provides; it raises the API limit from 60 to 5,000 requests per hour.
/// Only `api.github.com` ever receives it.
pub const ENV_GITHUB_TOKEN: &str = "ARTCRAFT_TOOLBOX_GITHUB_TOKEN";

pub struct Opened {
    pub session: Session,
    /// Problems worth telling the user about (unreadable files, no data directory).
    pub warnings: Vec<String>,
    /// What the first look at the installed apps changed ([`Session::rescan`]): an app that
    /// updated itself, an install that is gone.
    pub changes: Vec<String>,
}

/// The session for this user on this machine: the data directory from
/// [`artcraft_toolbox_store::dirs::data_dir`] and GitHub through [`github_client`].
pub fn open_user_session() -> Result<Opened> {
    let token = std::env::var(ENV_GITHUB_TOKEN).ok();
    open_in(data_dir(), Some(Arc::new(github_client(token))))
}

/// This user's data directory (`ARTCRAFT_TOOLBOX_CONFIG_DIR`, else the platform's), or why there
/// is none.
pub fn data_dir() -> std::result::Result<PathBuf, String> {
    artcraft_toolbox_store::dirs::data_dir().map(|(dir, _)| dir).map_err(|e| e.to_string())
}

/// A session in `dir` with `transport` (tests pass a temp dir and a fake). A directory that can't
/// be used is a warning: the session then saves nothing.
pub fn open_in(dir: std::result::Result<PathBuf, String>, transport: Option<Arc<dyn Transport>>) -> Result<Opened> {
    let catalog = Catalog::builtin()?;
    let mut warnings = Vec::new();
    let store = match dir.and_then(|d| Store::open(d).map_err(|e| e.to_string())) {
        Ok(store) => Some(store),
        Err(e) => {
            warnings.push(format!("{e}; nothing will be saved between runs"));
            None
        }
    };
    if let Some(store) = &store {
        log::info!("data directory: {}", store.root().display());
    }
    let (mut session, mut more) = Session::open(catalog, store, transport);
    session.use_platform_layout();
    session.use_process_install();
    let changes = session.rescan();
    warnings.append(&mut more);
    Ok(Opened { session, warnings, changes })
}

/// The GitHub client: [`Policy::github`], the toolbox's User-Agent, and `token` if given (sent to
/// the API only).
pub fn github_client(token: Option<String>) -> Client {
    Client::new(Policy::github(), &artcraft_toolbox_net::user_agent(build_info::VERSION), token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::temp;

    #[test]
    fn open_in_a_folder_and_without_one() {
        let root = temp("setup");
        let opened = open_in(Ok(root.clone()), None).unwrap();
        assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
        assert_eq!(opened.session.store().map(|s| s.root().to_path_buf()), Some(root.clone()));
        let blocked = temp("setup-blocked");
        std::fs::write(&blocked, "a file").unwrap();
        let opened = open_in(Ok(blocked.clone()), None).unwrap();
        assert!(opened.session.store().is_none());
        assert!(opened.warnings[0].contains("nothing will be saved"), "{:?}", opened.warnings);
        let opened = open_in(Err("no HOME".into()), None).unwrap();
        assert!(opened.warnings[0].starts_with("no HOME"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&blocked);
    }

    #[test]
    fn the_client_is_github_only() {
        let c = github_client(Some(" ghp_x ".into()));
        assert_eq!(c.policy(), &Policy::github());
        assert_eq!(c.policy().token_hosts, ["api.github.com"]);
        assert!(c.has_token());
        assert!(!github_client(Some("   ".into())).has_token());
    }
}
