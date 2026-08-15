//! App-owned filesystem locations that are needed without a Tauri `AppHandle`.

use std::path::PathBuf;

/// Tauri bundle identifier. This must match `tauri.conf.json`.
pub const APP_IDENTIFIER: &str = "com.beefapi.beefex";

/// Resolve the per-user Beefex app data directory with the same platform base
/// directory used by Tauri.
pub fn app_data_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|base| base.data_dir().join(APP_IDENTIFIER))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_data_dir_ends_with_beefex_identifier() {
        if let Some(dir) = app_data_dir() {
            assert_eq!(
                dir.file_name().and_then(|name| name.to_str()),
                Some(APP_IDENTIFIER)
            );
        }
    }

    #[test]
    fn identifier_matches_tauri_config() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri config JSON");
        assert_eq!(config["identifier"], APP_IDENTIFIER);
    }
}
