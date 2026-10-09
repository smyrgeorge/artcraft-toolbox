//! The desktop app's logger: `log::` records go to standard error.
//!
//! Levels: `info` for the toolbox's own crates (`artcraft_toolbox*`), `warn` for everything else
//! (wgpu is chatty). `RUST_LOG=<level>` sets the toolbox's own level (`debug`, `trace`, `off`).
//! A log file in the data directory with rotation, like PhotoCraft's `logging.rs`, comes with the
//! data directory itself (docs/roadmap.md M1).

use log::{LevelFilter, Log, Metadata, Record};

struct Logger {
    own: LevelFilter,
}

const OWN_PREFIX: &str = "artcraft_toolbox";

impl Logger {
    fn level_for(&self, target: &str) -> LevelFilter {
        if target.starts_with(OWN_PREFIX) { self.own } else { LevelFilter::Warn }
    }
}

impl Log for Logger {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= self.level_for(m.target())
    }

    fn log(&self, r: &Record) {
        if self.enabled(r.metadata()) {
            eprintln!("[{} {}] {}", r.level(), r.target(), r.args());
        }
    }

    fn flush(&self) {}
}

/// Parse `RUST_LOG` as one level; anything else keeps the default (`info`).
fn own_level(spec: Option<&str>) -> LevelFilter {
    spec.and_then(|s| s.trim().parse().ok()).unwrap_or(LevelFilter::Info)
}

pub fn init() {
    let own = own_level(std::env::var("RUST_LOG").ok().as_deref());
    // A second logger can't be installed; that's harmless (tests, embedders), so it's ignored.
    if log::set_boxed_logger(Box::new(Logger { own })).is_ok() {
        log::set_max_level(own.max(LevelFilter::Warn));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        assert_eq!(own_level(None), LevelFilter::Info);
        assert_eq!(own_level(Some("debug")), LevelFilter::Debug);
        assert_eq!(own_level(Some("nonsense")), LevelFilter::Info);
        let l = Logger { own: LevelFilter::Debug };
        assert_eq!(l.level_for("artcraft_toolbox_engine"), LevelFilter::Debug);
        assert_eq!(l.level_for("wgpu_core"), LevelFilter::Warn);
        let quiet = Logger { own: LevelFilter::Off };
        assert_eq!(quiet.level_for("wgpu_core"), LevelFilter::Warn);
    }
}
