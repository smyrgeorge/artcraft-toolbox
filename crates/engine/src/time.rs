//! The clock and human-readable durations ("5 min ago", "in 20 min") for the UI and messages.

use std::time::{SystemTime, UNIX_EPOCH};

/// Unix seconds now. A clock before 1970 reads as 0.
pub fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// How long ago `then` was: `just now`, `5 min ago`, `3 h ago`, `2 d ago`. A future `then` (a
/// clock moved back) reads as `just now`.
pub fn ago(now: u64, then: u64) -> String {
    match now.saturating_sub(then) {
        d if d < 60 => "just now".into(),
        d if d < 3600 => format!("{} min ago", d / 60),
        d if d < 86_400 => format!("{} h ago", d / 3600),
        d => format!("{} d ago", d / 86_400),
    }
}

/// How long until `then`, rounded up: `in under a minute`, `in 20 min`, `in 2 h`.
pub fn until(now: u64, then: u64) -> String {
    match then.saturating_sub(now) {
        d if d < 60 => "in under a minute".into(),
        d if d < 3600 => format!("in {} min", d.div_ceil(60)),
        d => format!("in {} h", d.div_ceil(3600)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(ago(1000, 1000), "just now");
        assert_eq!(ago(1000, 2000), "just now");
        assert_eq!(ago(1000 + 59, 1000), "just now");
        assert_eq!(ago(1000 + 5 * 60 + 30, 1000), "5 min ago");
        assert_eq!(ago(1000 + 3 * 3600, 1000), "3 h ago");
        assert_eq!(ago(1000 + 2 * 86_400, 1000), "2 d ago");
        assert_eq!(ago(u64::MAX, 0), format!("{} d ago", u64::MAX / 86_400));
        assert_eq!(until(1000, 1030), "in under a minute");
        assert_eq!(until(1000, 1000 + 61), "in 2 min");
        assert_eq!(until(1000, 1000 + 3600 * 2), "in 2 h");
        assert_eq!(until(2000, 1000), "in under a minute");
        assert!(now_unix() > 1_790_000_000);
    }
}
