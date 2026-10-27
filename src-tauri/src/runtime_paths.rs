//! Optional directories for isolated native acceptance runs.
//! Read once so analytics and persisted app state cannot choose different scopes.
use std::{ffi::OsString, path::PathBuf, sync::OnceLock};

#[derive(Debug, PartialEq, Eq)]
pub struct IsolatedDirectories {
    pub history: PathBuf,
    pub state: PathBuf,
}

fn parse(
    history: Option<OsString>,
    state: Option<OsString>,
) -> Result<Option<IsolatedDirectories>, String> {
    match (history, state) {
        (None, None) => Ok(None),
        (Some(history), Some(state)) => {
            let history = PathBuf::from(history);
            let state = PathBuf::from(state);
            if !history.is_absolute() || !state.is_absolute() {
                return Err("CODEX_USAGE_HISTORY_DIR 与 CODEX_USAGE_STATE_DIR 必须都是非空绝对目录路径".into());
            }
            Ok(Some(IsolatedDirectories { history, state }))
        }
        _ => Err("隔离验收必须同时提供 CODEX_USAGE_HISTORY_DIR 与 CODEX_USAGE_STATE_DIR；未使用不完整配置".into()),
    }
}

pub fn isolated_directories() -> Result<Option<&'static IsolatedDirectories>, String> {
    static DIRECTORIES: OnceLock<Result<Option<IsolatedDirectories>, String>> = OnceLock::new();
    DIRECTORIES
        .get_or_init(|| {
            parse(
                std::env::var_os("CODEX_USAGE_HISTORY_DIR"),
                std::env::var_os("CODEX_USAGE_STATE_DIR"),
            )
        })
        .as_ref()
        .map(Option::as_ref)
        .map_err(Clone::clone)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_overrides_preserve_normal_directory_resolution() {
        assert_eq!(parse(None, None).unwrap(), None);
    }

    #[test]
    fn accepts_a_complete_pair_of_absolute_directories() {
        let result = parse(
            Some("/tmp/native/history".into()),
            Some("/tmp/native/state".into()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(result.history, PathBuf::from("/tmp/native/history"));
        assert_eq!(result.state, PathBuf::from("/tmp/native/state"));
    }

    #[test]
    fn rejects_partial_empty_and_relative_overrides_instead_of_mixing_scopes() {
        for (history, state) in [
            (Some("/tmp/history"), None),
            (None, Some("/tmp/state")),
            (Some(""), Some("/tmp/state")),
            (Some("/tmp/history"), Some("")),
            (Some("history"), Some("/tmp/state")),
            (Some("/tmp/history"), Some("state")),
        ] {
            assert!(parse(history.map(OsString::from), state.map(OsString::from)).is_err());
        }
    }
}
