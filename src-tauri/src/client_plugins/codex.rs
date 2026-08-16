use crate::{
    app_paths,
    beefapi::{
        credential_store::{CredentialStore, FileCredentialStore, SecretCredential},
        types::AccountPhase,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};
use tauri::State;
use toml_edit::{value, Array, DocumentMut, Item, Table};

const CONFIG_FILE: &str = "config.toml";
const RECEIPT_VERSION: u8 = 2;
const MIN_CODEX_MINOR: u64 = 146;
const BEEFAPI_BASE_URL: &str = "https://beefapi.com/v1";
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct CodexPluginPaths {
    codex_home: PathBuf,
    config: PathBuf,
    data_root: PathBuf,
    credential: PathBuf,
    helper: PathBuf,
    receipt: PathBuf,
    backup: PathBuf,
}

impl CodexPluginPaths {
    fn new(codex_home: PathBuf, app_data: PathBuf) -> Self {
        let data_root = app_data.join("client-plugins").join("codex");
        Self {
            config: codex_home.join(CONFIG_FILE),
            credential: data_root.join("credentials").join("credential"),
            helper: data_root.join("credential.ps1"),
            receipt: data_root.join("receipt.json"),
            backup: data_root.join("config.backup"),
            codex_home,
            data_root,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPluginStatus {
    pub state: String,
    pub codex_version: Option<String>,
    pub supported: bool,
    pub codex_home: String,
    pub profile_path: String,
    pub credential_present: bool,
    pub configured_model: Option<String>,
    pub launch_command: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPluginChange {
    pub path: String,
    pub action: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPluginPreview {
    pub status: CodexPluginStatus,
    pub model: String,
    pub config_preview: String,
    pub changes: Vec<CodexPluginChange>,
    pub credential_contract: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexPluginReceipt {
    version: u8,
    config_path: String,
    config_sha256: String,
    model: String,
    codex_version: String,
    original_existed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPluginOperationReceipt {
    pub operation: String,
    pub status: CodexPluginStatus,
    pub changed_paths: Vec<String>,
    pub backup_path: Option<String>,
    pub config_valid: bool,
    pub doctor_summary: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct CodexDetection {
    version: Option<String>,
    supported: bool,
    reason: Option<String>,
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
fn sha256_hex(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn validate_model(model: &str) -> Result<String, String> {
    let model = model.trim();
    if model.is_empty()
        || model.len() > 128
        || !model
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || b"._:-".contains(&value))
    {
        return Err("codex_plugin_model_invalid".into());
    }
    Ok(model.to_string())
}

pub(crate) fn validate_server_allowed_model(
    state: &crate::state::AppState,
    model: &str,
) -> Result<(), String> {
    let account = state.beefapi_account.state();
    if account.phase != AccountPhase::SignedIn {
        return Err("codex_plugin_beefapi_sign_in_required".into());
    }
    if !account
        .allowed_models
        .iter()
        .any(|allowed| allowed == model)
    {
        return Err("codex_plugin_model_not_allowed".into());
    }
    Ok(())
}

fn parse_version(output: &str) -> Option<(String, u64, u64)> {
    let raw = output.split_whitespace().find(|part| {
        part.as_bytes().first().is_some_and(u8::is_ascii_digit) && part.contains('.')
    })?;
    let clean = raw.trim_matches(|value: char| !value.is_ascii_digit() && value != '.');
    let mut parts = clean.split('.');
    Some((
        clean.to_string(),
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ))
}

fn codex_version_command() -> Command {
    #[cfg(windows)]
    {
        let mut command = Command::new("cmd.exe");
        command.args(["/d", "/s", "/c", "codex --version"]);
        command
    }
    #[cfg(not(windows))]
    {
        let mut command = Command::new("codex");
        command.arg("--version");
        command
    }
}

pub(crate) fn detect_codex() -> CodexDetection {
    detection_from_output(codex_version_command().output().map(|output| {
        (
            output.status.success(),
            format!(
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    }))
}

fn detection_from_output(output: Result<(bool, String), std::io::Error>) -> CodexDetection {
    let combined = match output {
        Ok((true, output)) => output,
        Ok((false, _)) => {
            return CodexDetection {
                version: None,
                supported: false,
                reason: Some("codex_plugin_version_failed".into()),
            }
        }
        Err(_) => {
            return CodexDetection {
                version: None,
                supported: false,
                reason: Some("codex_plugin_missing".into()),
            }
        }
    };
    let Some((version, major, minor)) = parse_version(&combined) else {
        return CodexDetection {
            version: None,
            supported: false,
            reason: Some("codex_plugin_version_unrecognized".into()),
        };
    };
    let supported = major > 0 || minor >= MIN_CODEX_MINOR;
    CodexDetection {
        version: Some(version),
        supported,
        reason: (!supported).then(|| "codex_plugin_version_unsupported".into()),
    }
}

fn validate_root(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{label}_relative"));
    }
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!("{label}_unsafe"));
        }
    }
    Ok(path)
}

pub(crate) fn resolve_paths() -> Result<CodexPluginPaths, String> {
    let codex_home = env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            directories::BaseDirs::new()
                .map(|dirs| dirs.home_dir().join(".codex"))
                .unwrap_or_default()
        });
    let app_data =
        app_paths::app_data_dir().ok_or_else(|| "codex_plugin_app_data_unavailable".to_string())?;
    Ok(CodexPluginPaths::new(
        validate_root(codex_home, "codex_plugin_home")?,
        validate_root(app_data, "codex_plugin_app_data")?,
    ))
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("codex_plugin_config_read_failed".into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_CONFIG_BYTES
    {
        return Err("codex_plugin_config_read_failed".into());
    }
    fs::read(path)
        .map(Some)
        .map_err(|_| "codex_plugin_config_read_failed".into())
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "codex_plugin_config_write_failed".to_string())?;
    fs::create_dir_all(parent).map_err(|_| "codex_plugin_config_write_failed".to_string())?;
    let temporary = parent.join(format!(".beefex-managed.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|_| "codex_plugin_config_write_failed".to_string())?;
        file.write_all(content)
            .map_err(|_| "codex_plugin_config_write_failed".to_string())?;
        file.sync_all()
            .map_err(|_| "codex_plugin_config_write_failed".to_string())?;
        drop(file);
        fs::rename(&temporary, path).map_err(|_| "codex_plugin_config_write_failed".to_string())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn credential_command(paths: &CodexPluginPaths) -> (String, Vec<String>) {
    if cfg!(windows) {
        (
            "powershell.exe".into(),
            vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                path_string(&paths.helper),
                path_string(&paths.credential),
            ],
        )
    } else {
        ("/bin/cat".into(), vec![path_string(&paths.credential)])
    }
}

fn merged_config(
    paths: &CodexPluginPaths,
    original: Option<&[u8]>,
    model: &str,
) -> Result<String, String> {
    let source = original
        .map(|bytes| {
            std::str::from_utf8(bytes).map_err(|_| "codex_plugin_config_invalid".to_string())
        })
        .transpose()?
        .unwrap_or("");
    let mut document = if source.trim().is_empty() {
        DocumentMut::new()
    } else {
        source
            .parse::<DocumentMut>()
            .map_err(|_| "codex_plugin_config_invalid".to_string())?
    };
    match document.get("model_providers") {
        Some(item) if !item.is_table() => return Err("codex_plugin_config_conflict".into()),
        None => document["model_providers"] = Item::Table(Table::new()),
        Some(_) => {}
    }
    document["model"] = value(model);
    document["model_provider"] = value("beefapi");
    let (command, args) = credential_command(paths);
    let mut auth = Table::new();
    auth["command"] = value(command);
    let mut auth_args = Array::new();
    for argument in args {
        auth_args.push(argument);
    }
    auth["args"] = value(auth_args);
    auth["timeout_ms"] = value(5000);
    auth["refresh_interval_ms"] = value(0);
    let mut provider = Table::new();
    provider["name"] = value("BeefAPI");
    provider["base_url"] = value(BEEFAPI_BASE_URL);
    provider["wire_api"] = value("responses");
    provider["auth"] = Item::Table(auth);
    document["model_providers"]
        .as_table_mut()
        .expect("model_providers normalized above")
        .insert("beefapi", Item::Table(provider));
    Ok(document.to_string())
}

fn read_receipt(paths: &CodexPluginPaths) -> Result<Option<CodexPluginReceipt>, String> {
    let Some(bytes) = read_bounded(&paths.receipt)? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "codex_plugin_receipt_invalid".into())
}

fn write_receipt(paths: &CodexPluginPaths, receipt: &CodexPluginReceipt) -> Result<(), String> {
    atomic_write(
        &paths.receipt,
        &serde_json::to_vec_pretty(receipt)
            .map_err(|_| "codex_plugin_receipt_write_failed".to_string())?,
    )
}

fn configured_model(bytes: &[u8]) -> Option<String> {
    let doc = std::str::from_utf8(bytes)
        .ok()?
        .parse::<DocumentMut>()
        .ok()?;
    if doc["model_provider"].as_str() != Some("beefapi") {
        return None;
    }
    doc["model"].as_str().map(str::to_string)
}

fn inspect_with(
    paths: &CodexPluginPaths,
    detection: CodexDetection,
) -> Result<CodexPluginStatus, String> {
    let config = read_bounded(&paths.config)?;
    let receipt = read_receipt(paths)?;
    let credential_present = FileCredentialStore::new(paths.credential.clone())
        .read()?
        .is_some();
    let model = config.as_deref().and_then(configured_model);
    let (state, reason) = if !detection.supported {
        (
            if detection.version.is_some() {
                "unsupported"
            } else {
                "missing"
            }
            .to_string(),
            detection.reason.clone(),
        )
    } else if let (Some(config), Some(receipt)) = (config.as_deref(), receipt.as_ref()) {
        if receipt.version == RECEIPT_VERSION
            && sha256_hex(config) == receipt.config_sha256
            && credential_present
            && model.is_some()
        {
            ("configured".into(), None)
        } else {
            (
                "conflict".into(),
                Some("codex_plugin_config_changed".into()),
            )
        }
    } else if model.is_some() {
        (
            "conflict".into(),
            Some("codex_plugin_unmanaged_beefapi_config".into()),
        )
    } else {
        ("ready".into(), None)
    };
    Ok(CodexPluginStatus {
        state,
        codex_version: detection.version,
        supported: detection.supported,
        codex_home: path_string(&paths.codex_home),
        profile_path: path_string(&paths.config),
        credential_present,
        configured_model: model,
        launch_command: "codex".into(),
        reason,
    })
}

#[tauri::command]
pub fn codex_plugin_inspect() -> Result<CodexPluginStatus, String> {
    inspect_with(&resolve_paths()?, detect_codex())
}

#[tauri::command]
pub fn codex_plugin_preview(model: String) -> Result<CodexPluginPreview, String> {
    let paths = resolve_paths()?;
    let model = validate_model(&model)?;
    let original = read_bounded(&paths.config)?;
    Ok(CodexPluginPreview {
        status: inspect_with(&paths, detect_codex())?, model: model.clone(), config_preview: merged_config(&paths, original.as_deref(), &model)?,
        changes: vec![
            CodexPluginChange { path: path_string(&paths.config), action: "merge".into(), description: "Set BeefAPI as the normal Codex default while preserving unrelated settings.".into() },
            CodexPluginChange { path: path_string(&paths.credential), action: "write".into(), description: "Store the managed gpt-pro key in a Beefex owner-only file.".into() },
        ],
        credential_contract: "Uses the current Beefex login to create or reuse a managed gpt-pro key. No token paste and no extra login.".into(),
    })
}

fn windows_helper_text() -> &'static str {
    "$ErrorActionPreference = 'Stop'\n$value = [System.IO.File]::ReadAllText($args[0]).Trim()\nif ([string]::IsNullOrWhiteSpace($value)) { throw 'empty credential' }\n[Console]::Out.Write($value)\n"
}

pub(crate) async fn apply_managed(
    paths: &CodexPluginPaths,
    model: &str,
    credential: &SecretCredential,
    detection: CodexDetection,
) -> Result<CodexPluginOperationReceipt, String> {
    if !detection.supported {
        return Err(detection
            .reason
            .unwrap_or_else(|| "codex_plugin_unsupported".into()));
    }
    fs::create_dir_all(&paths.codex_home)
        .map_err(|_| "codex_plugin_home_create_failed".to_string())?;
    fs::create_dir_all(&paths.data_root)
        .map_err(|_| "codex_plugin_data_create_failed".to_string())?;
    let original = read_bounded(&paths.config)?;
    if read_receipt(paths)?.is_none() {
        if let Some(bytes) = original.as_deref() {
            atomic_write(&paths.backup, bytes)?;
        }
    }
    FileCredentialStore::new(paths.credential.clone()).write(credential)?;
    if cfg!(windows) {
        atomic_write(&paths.helper, windows_helper_text().as_bytes())?;
    }
    let merged = merged_config(paths, original.as_deref(), model)?;
    atomic_write(&paths.config, merged.as_bytes())?;
    let readback = read_bounded(&paths.config)?
        .ok_or_else(|| "codex_plugin_config_readback_missing".to_string())?;
    if readback != merged.as_bytes() {
        return Err("codex_plugin_config_readback_mismatch".into());
    }
    let receipt = CodexPluginReceipt {
        version: RECEIPT_VERSION,
        config_path: path_string(&paths.config),
        config_sha256: sha256_hex(&readback),
        model: model.into(),
        codex_version: detection.version.clone().unwrap_or_default(),
        original_existed: original.is_some(),
    };
    write_receipt(paths, &receipt)?;
    Ok(CodexPluginOperationReceipt {
        operation: "apply".into(),
        status: inspect_with(paths, detection)?,
        changed_paths: vec![path_string(&paths.config), path_string(&paths.credential)],
        backup_path: receipt.original_existed.then(|| path_string(&paths.backup)),
        config_valid: true,
        doctor_summary: None,
    })
}

#[tauri::command]
pub async fn codex_plugin_apply(
    state: State<'_, crate::state::AppState>,
    model: String,
) -> Result<CodexPluginOperationReceipt, String> {
    let model = validate_model(&model)?;
    validate_server_allowed_model(&state, &model)?;
    let credentials = state.beefapi_account.ensure_client_credentials().await?;
    if credentials.gpt.group != "gpt-pro" {
        return Err("codex_plugin_managed_group_invalid".into());
    }
    apply_managed(
        &resolve_paths()?,
        &model,
        &credentials.gpt.credential,
        detect_codex(),
    )
    .await
}

#[tauri::command]
pub fn codex_plugin_verify() -> Result<CodexPluginOperationReceipt, String> {
    let paths = resolve_paths()?;
    let status = inspect_with(&paths, detect_codex())?;
    if status.state != "configured" {
        return Err(status
            .reason
            .unwrap_or_else(|| "codex_plugin_not_configured".into()));
    }
    let output = Command::new("codex")
        .arg("--strict-config")
        .arg("--version")
        .output()
        .map_err(|_| "codex_plugin_doctor_failed".to_string())?;
    if !output.status.success() {
        return Err("codex_plugin_doctor_failed".into());
    }
    Ok(CodexPluginOperationReceipt {
        operation: "verify".into(),
        status,
        changed_paths: vec![],
        backup_path: None,
        config_valid: true,
        doctor_summary: Some(String::from_utf8_lossy(&output.stdout).trim().to_string()),
    })
}

#[tauri::command]
pub fn codex_plugin_rollback() -> Result<CodexPluginOperationReceipt, String> {
    let paths = resolve_paths()?;
    let receipt = read_receipt(&paths)?.ok_or_else(|| "codex_plugin_not_configured".to_string())?;
    let current = read_bounded(&paths.config)?
        .ok_or_else(|| "codex_plugin_config_readback_missing".to_string())?;
    if sha256_hex(&current) != receipt.config_sha256 {
        return Err("codex_plugin_config_changed".into());
    }
    if receipt.original_existed {
        atomic_write(
            &paths.config,
            &read_bounded(&paths.backup)?
                .ok_or_else(|| "codex_plugin_backup_missing".to_string())?,
        )?;
    } else {
        fs::remove_file(&paths.config)
            .map_err(|_| "codex_plugin_config_delete_failed".to_string())?;
    }
    FileCredentialStore::new(paths.credential.clone()).delete()?;
    for path in [&paths.helper, &paths.receipt, &paths.backup] {
        let _ = fs::remove_file(path);
    }
    Ok(CodexPluginOperationReceipt {
        operation: "rollback".into(),
        status: inspect_with(&paths, detect_codex())?,
        changed_paths: vec![path_string(&paths.config), path_string(&paths.credential)],
        backup_path: None,
        config_valid: true,
        doctor_summary: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_paths(label: &str) -> CodexPluginPaths {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("beefex-codex-v2-{label}-{nonce}"));
        fs::create_dir_all(root.join("codex")).unwrap();
        fs::create_dir_all(root.join("app")).unwrap();
        CodexPluginPaths::new(root.join("codex"), root.join("app"))
    }
    fn supported() -> CodexDetection {
        CodexDetection {
            version: Some("0.146.0".into()),
            supported: true,
            reason: None,
        }
    }

    #[test]
    fn merge_preserves_unrelated_config_and_sets_normal_default() {
        let paths = fixture_paths("merge");
        let merged = merged_config(
            &paths,
            Some(b"approval_policy = \"on-request\"\n[features]\nweb_search = true\n"),
            "gpt-5.6-sol",
        )
        .unwrap();
        assert!(merged.contains("approval_policy = \"on-request\""));
        assert!(merged.contains("web_search = true"));
        assert!(merged.contains("model_provider = \"beefapi\""));
        assert!(merged.contains("[model_providers.beefapi]"));
        assert!(!merged.contains("--profile"));
    }

    #[tokio::test]
    async fn apply_is_idempotent_and_preserves_original_backup() {
        let paths = fixture_paths("roundtrip");
        let original = b"approval_policy = \"on-request\"\n";
        atomic_write(&paths.config, original).unwrap();
        let secret = SecretCredential::new("sk-managed-test".into());
        assert_eq!(
            apply_managed(&paths, "gpt-5.6-sol", &secret, supported())
                .await
                .unwrap()
                .status
                .state,
            "configured"
        );
        assert_eq!(
            apply_managed(&paths, "gpt-5.6-sol", &secret, supported())
                .await
                .unwrap()
                .status
                .state,
            "configured"
        );
        assert_eq!(read_bounded(&paths.backup).unwrap().unwrap(), original);
    }

    #[test]
    fn secret_debug_is_redacted() {
        assert_eq!(
            format!("{:?}", SecretCredential::new("sk-secret".into())),
            "<redacted>"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_codex_detection_uses_the_cmd_shim() {
        let command = codex_version_command();
        assert_eq!(command.get_program(), "cmd.exe");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["/d", "/s", "/c", "codex --version"]
        );
    }
}
