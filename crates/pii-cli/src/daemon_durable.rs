//! Where a daemon's durable stores live when it was given an explicit `--store` (spec 418).
//!
//! Measured 2026-11-12: a throwaway daemon started with `--store "$D/store.redb" --workspace "$D"` opened
//! `/root/.local/state/pii/durable/channel.redb` — the operator's channel store — and only failed because that file
//! was busy. On a quiet machine the second instance would have opened the operator's state and advanced its cursors.
//! `PII_STATE_DIR` was the workaround, and it is not something a user of `--store` should have to know.
//!
//! An explicit `--store` means "a separate instance", so its durable stores belong beside that store. The decision is
//! expressed through `[core].durable_dir`, the field the daemon's single resolution rule already reads, so nothing in
//! the daemon learns a second precedence chain.

use std::path::{Path, PathBuf};

use pii_server::daemon::config::Config;

/// The store path, the directory that holds it, and whether the flag was given at all.
///
/// The flag matters because only an EXPLICIT `--store` means "a separate instance": the default lives in the runtime
/// dir, which spec 337 forbids as a home for durable state.
pub fn store_and_parent(store: Option<PathBuf>, runtime_dir: &Path) -> (PathBuf, PathBuf, bool) {
    let given = store.is_some();
    let path = store.unwrap_or_else(|| runtime_dir.join("store.redb"));
    let parent = path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    (path, parent, given)
}

/// The durable directory for this daemon, after recording the override on `cfg` so the daemon's own resolution agrees
/// with this one.
pub fn durable_dir(cfg: &mut Config, store_given: bool, store_parent: &Path) -> PathBuf {
    if let Some(over) = store_override(cfg, store_given, store_parent, &pii_core::env_var) {
        cfg.core.durable_dir = Some(over.display().to_string());
    }
    cfg.resolve_durable_dir(&pii_core::state_dir())
}

/// The override an explicit `--store` implies, or `None` when it implies nothing.
///
/// An explicit `[core].durable_dir` is never overridden from a command line, and `PII_STATE_DIR` already moves every
/// durable store by itself, so the flag only speaks when neither does.
fn store_override(
    cfg: &Config,
    store_given: bool,
    store_parent: &Path,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    if !store_given || cfg.core.durable_dir.is_some() {
        return None;
    }
    if env("PII_STATE_DIR").is_some_and(|s| !s.is_empty()) {
        return None;
    }
    if store_parent.as_os_str().is_empty() {
        return None;
    }
    Some(store_parent.join("durable"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    #![allow(non_snake_case)]
    // reason: a test module; the CONTROL_ names are the spec's contract, and a mismatch is the test failing.

    use super::*;

    fn env_with(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k: &str| {
            pairs
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
        }
    }

    fn cfg_with_durable(value: Option<&str>) -> Config {
        let mut cfg = Config::default();
        cfg.core.durable_dir = value.map(str::to_owned);
        cfg
    }

    #[test]
    fn an_explicit_store_puts_the_durable_dir_beside_it() {
        let got = store_override(
            &cfg_with_durable(None),
            true,
            Path::new("/tmp/x"),
            &env_with(&[("HOME", "/home/op")]),
        );
        assert_eq!(got, Some(PathBuf::from("/tmp/x/durable")));
    }

    #[test]
    fn CONTROL_pii_state_dir_still_wins_over_the_store() {
        let got = store_override(
            &cfg_with_durable(None),
            true,
            Path::new("/tmp/x"),
            &env_with(&[("PII_STATE_DIR", "/srv/pii-state")]),
        );
        assert_eq!(got, None, "the state dir already moves every durable store");
    }

    #[test]
    fn CONTROL_a_configured_durable_dir_wins_over_the_store() {
        let got = store_override(
            &cfg_with_durable(Some("/srv/chosen")),
            true,
            Path::new("/tmp/x"),
            &env_with(&[]),
        );
        assert_eq!(got, None, "an explicit configuration is never overridden");
    }

    #[test]
    fn CONTROL_no_store_means_no_override() {
        let got = store_override(
            &cfg_with_durable(None),
            false,
            Path::new("/tmp/x"),
            &env_with(&[]),
        );
        assert_eq!(got, None, "the default store lives in the runtime dir");
    }

    #[test]
    fn CONTROL_an_empty_pii_state_dir_is_not_a_value() {
        let got = store_override(
            &cfg_with_durable(None),
            true,
            Path::new("/tmp/x"),
            &env_with(&[("PII_STATE_DIR", "")]),
        );
        assert_eq!(got, Some(PathBuf::from("/tmp/x/durable")));
    }

    #[test]
    fn CONTROL_a_store_with_no_parent_changes_nothing() {
        let got = store_override(&cfg_with_durable(None), true, Path::new(""), &env_with(&[]));
        assert_eq!(got, None);
    }
}
