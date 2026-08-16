use crate::{
    app_paths,
    beefapi::credential_store::{CredentialStore, FileCredentialStore, SecretCredential},
    beefapi::types::AccountPhase,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

const PROFILE_NAME: &str = "beefapi";
const PROFILE_FILE: &str = "beefapi.config.toml";
const OWNERSHIP_MARKER: &str = "# beefex-managed-codex-profile-v1";
const RECEIPT_VERSION: u8 = 1;
const MIN_CODEX_MINOR: u64 = 146;
const BEEFAPI_BASE_URL: &str = "https://beefapi.com/v1";
const MAX_MANAGED_FILE_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug)]
struct CodexPluginPaths {
    codex_home: PathBuf,
    profile: PathBuf,
    data_root: PathBuf,
    credential: PathBuf,
    helper: PathBuf,
    receipt: PathBuf,
    backup: PathBuf,
    credential_backup: PathBuf,
}

impl CodexPluginPaths {
    fn new(codex_home: PathBuf, app_data: PathBuf) -> Self {
        let data_root = app_data.join("client-plugins").join("codex");
        Self {
            profile: codex_home.join(PROFILE_FILE),
            credential: data_root.join("credential"),
            helper: data_root.join("credential.ps1"),
            receipt: data_root.join("receipt.json"),
            backup: data_root.join("profile.backup"),
            credential_backup: data_root.join("credential.backup"),
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
    profile_path: String,
    profile_sha256: String,
    model: String,
    codex_version: String,
    backup_present: bool,
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
struct CodexDetection {
    version: Option<String>,
    supported: bool,
    reason: Option<String>,
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn sha256_hex(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn validate_model(model: &str) -> Result<String, String> {
    let model = model.trim();
    if model.is_empty()
        || model.len() > 128
        || !model
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || b"._:-".contains(&value))
    {
        return Err("codex_plugin_model_invalid".to_string());
    }
    Ok(model.to_string())
}

fn validate_server_allowed_model(
    state: &crate::state::AppState,
    model: &str,
) -> Result<(), String> {
    let account = state.beefapi_account.state();
    if account.phase != AccountPhase::SignedIn {
        return Err("codex_plugin_beefapi_sign_in_required".to_string());
    }
    if !account
        .allowed_models
        .iter()
        .any(|allowed| allowed == model)
    {
        return Err("codex_plugin_model_not_allowed".to_string());
    }
    Ok(())
}

fn validate_token(token: &str) -> Result<SecretCredential, String> {
    let token = token.trim();
    if token.len() < 8 || token.len() > 64 * 1024 || token.contains(['\r', '\n', '\0']) {
        return Err("codex_plugin_credential_invalid".to_string());
    }
    Ok(SecretCredential::new(token.to_string()))
}

fn toml_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            value if value.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", value as u32));
            }
            value => escaped.push(value),
        }
    }
    escaped.push('"');
    escaped
}

fn profile_text(paths: &CodexPluginPaths, model: &str) -> String {
    let (command, args) = if cfg!(windows) {
        (
            "powershell.exe".to_string(),
            vec![
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                path_string(&paths.helper),
                path_string(&paths.credential),
            ],
        )
    } else {
        ("/bin/cat".to_string(), vec![path_string(&paths.credential)])
    };
    let args = args
        .iter()
        .map(|value| toml_string(value))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{OWNERSHIP_MARKER}\nmodel = {}\nmodel_provider = \"beefapi\"\n\n[model_providers.beefapi]\nname = \"BeefAPI\"\nbase_url = \"{BEEFAPI_BASE_URL}\"\nwire_api = \"responses\"\n\n[model_providers.beefapi.auth]\ncommand = {}\nargs = [{args}]\ntimeout_ms = 5000\nrefresh_interval_ms = 0\n",
        toml_string(model),
        toml_string(&command),
    )
}

fn windows_helper_text() -> &'static str {
    "$ErrorActionPreference = 'Stop'\n$path = $args[0]\nif ([string]::IsNullOrWhiteSpace($path)) { throw 'missing credential path' }\n$value = [System.IO.File]::ReadAllText($path).Trim()\nif ([string]::IsNullOrWhiteSpace($value)) { throw 'empty credential' }\n[Console]::Out.Write($value)\n"
}

fn parse_version(output: &str) -> Option<(String, u64, u64, u64)> {
    let raw = output.split_whitespace().find(|part| {
        part.as_bytes()
            .first()
            .is_some_and(|value| value.is_ascii_digit())
            && part.contains('.')
    })?;
    let clean = raw.trim_matches(|value: char| !value.is_ascii_digit() && value != '.');
    let mut parts = clean.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()
        .unwrap_or("0")
        .trim_matches(|value: char| !value.is_ascii_digit())
        .parse()
        .ok()?;
    Some((clean.to_string(), major, minor, patch))
}

fn detect_codex() -> CodexDetection {
    let output = Command::new("codex").arg("--version").output();
    let output = match output {
        Ok(output) => Ok((
            output.status.success(),
            format!(
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        )),
        Err(_) => Err(()),
    };
    detection_from_output(output)
}

fn detection_from_output(output: Result<(bool, String), ()>) -> CodexDetection {
    let combined = match output {
        Ok((true, output)) => output,
        Ok((false, _)) => {
            return CodexDetection {
                version: None,
                supported: false,
                reason: Some("codex_plugin_version_failed".to_string()),
            }
        }
        Err(()) => {
            return CodexDetection {
                version: None,
                supported: false,
                reason: Some("codex_plugin_missing".to_string()),
            }
        }
    };
    let Some((version, major, minor, _patch)) = parse_version(&combined) else {
        return CodexDetection {
            version: None,
            supported: false,
            reason: Some("codex_plugin_version_unrecognized".to_string()),
        };
    };
    let supported = major > 0 || minor >= MIN_CODEX_MINOR;
    CodexDetection {
        version: Some(version),
        supported,
        reason: (!supported).then(|| "codex_plugin_version_unsupported".to_string()),
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

fn resolve_paths() -> Result<CodexPluginPaths, String> {
    let codex_home = match env::var_os("CODEX_HOME") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => directories::BaseDirs::new()
            .ok_or_else(|| "codex_plugin_home_unavailable".to_string())?
            .home_dir()
            .join(".codex"),
    };
    let app_data =
        app_paths::app_data_dir().ok_or_else(|| "codex_plugin_app_data_unavailable".to_string())?;
    Ok(CodexPluginPaths::new(
        validate_root(codex_home, "codex_plugin_home")?,
        validate_root(app_data, "codex_plugin_app_data")?,
    ))
}

fn secure_store(path: &Path) -> FileCredentialStore {
    FileCredentialStore::new(path.to_path_buf())
}

fn read_secure_text(path: &Path) -> Result<Option<String>, String> {
    secure_store(path)
        .read()
        .map(|value| value.map(|secret| secret.expose().to_string()))
}

fn write_secure_text(path: &Path, value: &str) -> Result<(), String> {
    secure_store(path).write(&SecretCredential::new(value.to_string()))
}

fn delete_secure_file(path: &Path) -> Result<(), String> {
    secure_store(path).delete()
}

fn read_managed_profile(path: &Path) -> Result<Option<String>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("codex_plugin_profile_read_failed".to_string()),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_MANAGED_FILE_BYTES
    {
        return Err("codex_plugin_profile_read_failed".to_string());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| {
            file.take(MAX_MANAGED_FILE_BYTES + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| "codex_plugin_profile_read_failed".to_string())?;
    if bytes.len() as u64 > MAX_MANAGED_FILE_BYTES {
        return Err("codex_plugin_profile_read_failed".to_string());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "codex_plugin_profile_read_failed".to_string())
}

fn write_managed_profile(path: &Path, value: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "codex_plugin_profile_write_failed".to_string())?;
    fs::create_dir_all(parent).map_err(|_| "codex_plugin_profile_write_failed".to_string())?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|_| "codex_plugin_profile_write_failed".to_string())?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err("codex_plugin_profile_write_failed".to_string());
    }
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("codex_plugin_profile_write_failed".to_string());
        }
    }
    let temporary = parent.join(format!(
        ".beefex-codex-profile.{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        #[cfg(unix)]
        let mut file = {
            use std::os::unix::fs::OpenOptionsExt;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
                .map_err(|_| "codex_plugin_profile_write_failed".to_string())?
        };
        #[cfg(not(unix))]
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| "codex_plugin_profile_write_failed".to_string())?;
        file.write_all(value.as_bytes())
            .map_err(|_| "codex_plugin_profile_write_failed".to_string())?;
        file.sync_all()
            .map_err(|_| "codex_plugin_profile_write_failed".to_string())?;
        drop(file);
        #[cfg(windows)]
        crate::beefapi::credential_store::atomic_replace_windows(&temporary, path)
            .map_err(|_| "codex_plugin_profile_write_failed".to_string())?;
        #[cfg(not(windows))]
        fs::rename(&temporary, path)
            .map_err(|_| "codex_plugin_profile_write_failed".to_string())?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| "codex_plugin_profile_write_failed".to_string())?;
        let readback = read_managed_profile(path)?
            .ok_or_else(|| "codex_plugin_profile_write_failed".to_string())?;
        if readback.as_bytes() != value.as_bytes() {
            return Err("codex_plugin_profile_write_failed".to_string());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn delete_managed_profile(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err("codex_plugin_profile_delete_failed".to_string())
        }
        Ok(_) => {
            fs::remove_file(path).map_err(|_| "codex_plugin_profile_delete_failed".to_string())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("codex_plugin_profile_delete_failed".to_string()),
    }
}

fn read_profile(paths: &CodexPluginPaths) -> Result<Option<String>, String> {
    read_managed_profile(&paths.profile)
}

fn parse_owned_profile(content: &str) -> Result<Option<String>, String> {
    if !content
        .lines()
        .next()
        .is_some_and(|line| line == OWNERSHIP_MARKER)
    {
        return Err("codex_plugin_profile_conflict".to_string());
    }
    let value: toml::Value =
        toml::from_str(content).map_err(|_| "codex_plugin_profile_invalid".to_string())?;
    let provider = value
        .get("model_providers")
        .and_then(|value| value.get("beefapi"));
    if value.get("model_provider").and_then(toml::Value::as_str) != Some("beefapi")
        || provider
            .and_then(|value| value.get("base_url"))
            .and_then(toml::Value::as_str)
            != Some(BEEFAPI_BASE_URL)
        || provider
            .and_then(|value| value.get("wire_api"))
            .and_then(toml::Value::as_str)
            != Some("responses")
    {
        return Err("codex_plugin_profile_invalid".to_string());
    }
    Ok(value
        .get("model")
        .and_then(toml::Value::as_str)
        .map(str::to_string))
}

fn read_receipt(paths: &CodexPluginPaths) -> Result<Option<CodexPluginReceipt>, String> {
    let Some(content) = read_secure_text(&paths.receipt)
        .map_err(|_| "codex_plugin_receipt_read_failed".to_string())?
    else {
        return Ok(None);
    };
    serde_json::from_str(&content)
        .map(Some)
        .map_err(|_| "codex_plugin_receipt_invalid".to_string())
}

fn inspect_with(paths: &CodexPluginPaths, detection: CodexDetection) -> CodexPluginStatus {
    let mut state = if detection.version.is_none() {
        "missing"
    } else if !detection.supported {
        "unsupported"
    } else {
        "ready"
    }
    .to_string();
    let mut reason = detection.reason.clone();
    let mut configured_model = None;
    let credential_present = secure_store(&paths.credential)
        .read()
        .map(|value| value.is_some())
        .unwrap_or(false);

    match read_profile(paths) {
        Ok(Some(content)) => match parse_owned_profile(&content) {
            Ok(model) => {
                configured_model = model;
                if !credential_present {
                    state = "failed".to_string();
                    reason = Some("codex_plugin_credential_missing".to_string());
                } else if let Ok(Some(receipt)) = read_receipt(paths) {
                    if receipt.profile_sha256 == sha256_hex(&content) {
                        state = "configured".to_string();
                        reason = None;
                    } else {
                        state = "failed".to_string();
                        reason = Some("codex_plugin_profile_readback_mismatch".to_string());
                    }
                } else {
                    state = "failed".to_string();
                    reason = Some("codex_plugin_receipt_missing".to_string());
                }
            }
            Err(error) if error == "codex_plugin_profile_conflict" => {
                state = "conflict".to_string();
                reason = Some(error);
            }
            Err(error) => {
                state = "failed".to_string();
                reason = Some(error);
            }
        },
        Ok(None) => {}
        Err(error) => {
            state = "failed".to_string();
            reason = Some(error);
        }
    }

    CodexPluginStatus {
        state,
        codex_version: detection.version,
        supported: detection.supported,
        codex_home: path_string(&paths.codex_home),
        profile_path: path_string(&paths.profile),
        credential_present,
        configured_model,
        launch_command: format!("codex --profile {PROFILE_NAME}"),
        reason,
    }
}

fn inspect(paths: &CodexPluginPaths) -> CodexPluginStatus {
    inspect_with(paths, detect_codex())
}

fn preview_with(paths: &CodexPluginPaths, model: &str) -> Result<CodexPluginPreview, String> {
    let model = validate_model(model)?;
    let status = inspect(paths);
    if matches!(status.state.as_str(), "conflict" | "failed") {
        return Err(status
            .reason
            .clone()
            .unwrap_or_else(|| "codex_plugin_not_ready".to_string()));
    }
    let profile = profile_text(paths, &model);
    let profile_action = if status.state == "configured" {
        "replace"
    } else {
        "create"
    };
    let mut changes = vec![
        CodexPluginChange {
            path: path_string(&paths.credential),
            action: "write_private".to_string(),
            description: "Store the separately created BeefAPI API token; its value is never written to receipts.".to_string(),
        },
        CodexPluginChange {
            path: path_string(&paths.profile),
            action: profile_action.to_string(),
            description: "Install the isolated BeefAPI Codex profile without editing config.toml.".to_string(),
        },
        CodexPluginChange {
            path: path_string(&paths.receipt),
            action: "write_private".to_string(),
            description: "Record paths, version, model, hashes and rollback target without credentials.".to_string(),
        },
    ];
    if cfg!(windows) {
        changes.insert(
            1,
            CodexPluginChange {
                path: path_string(&paths.helper),
                action: "write_private".to_string(),
                description:
                    "Install the fixed credential reader used by Codex command-backed auth."
                        .to_string(),
            },
        );
    }
    Ok(CodexPluginPreview {
        status,
        model,
        config_preview: profile,
        changes,
        credential_contract: "A separate user-created BeefAPI API token is stored in a Beefex-owned private file. The Beefex desktop session credential is never read or exported.".to_string(),
    })
}

fn restore_secure(path: &Path, value: Option<&str>) {
    if let Some(value) = value {
        let _ = write_secure_text(path, value);
    } else {
        let _ = delete_secure_file(path);
    }
}

fn restore_profile(path: &Path, value: Option<&str>) {
    if let Some(value) = value {
        let _ = write_managed_profile(path, value);
    } else {
        let _ = delete_managed_profile(path);
    }
}

fn restore_secret(path: &Path, value: Option<&SecretCredential>) {
    if let Some(value) = value {
        let _ = secure_store(path).write(value);
    } else {
        let _ = delete_secure_file(path);
    }
}

fn apply_with(
    paths: &CodexPluginPaths,
    model: &str,
    token: &str,
    detection: CodexDetection,
) -> Result<CodexPluginOperationReceipt, String> {
    let model = validate_model(model)?;
    let token = validate_token(token)?;
    if !detection.supported {
        return Err(detection
            .reason
            .clone()
            .unwrap_or_else(|| "codex_plugin_unsupported".to_string()));
    }
    let existing_profile = read_profile(paths)?;
    if let Some(content) = existing_profile.as_deref() {
        parse_owned_profile(content)?;
    }
    let existing_token = secure_store(&paths.credential).read()?;
    let existing_receipt = read_secure_text(&paths.receipt)?;
    let existing_helper = read_secure_text(&paths.helper)?;
    let existing_backup = read_secure_text(&paths.backup)?;
    let existing_credential_backup = secure_store(&paths.credential_backup).read()?;
    let profile = profile_text(paths, &model);
    toml::from_str::<toml::Value>(&profile)
        .map_err(|_| "codex_plugin_profile_generation_failed".to_string())?;

    let apply_result = (|| {
        fs::create_dir_all(&paths.codex_home)
            .map_err(|_| "codex_plugin_home_create_failed".to_string())?;
        secure_store(&paths.credential).write(&token)?;
        if cfg!(windows) {
            write_secure_text(&paths.helper, windows_helper_text())?;
        }
        if let Some(content) = existing_profile.as_deref() {
            write_secure_text(&paths.backup, content)?;
            if let Some(existing_token) = existing_token.as_ref() {
                secure_store(&paths.credential_backup).write(existing_token)?;
            } else {
                delete_secure_file(&paths.credential_backup)?;
            }
        } else {
            delete_secure_file(&paths.backup)?;
            delete_secure_file(&paths.credential_backup)?;
        }
        write_managed_profile(&paths.profile, &profile)?;
        let readback = read_profile(paths)?
            .ok_or_else(|| "codex_plugin_profile_readback_missing".to_string())?;
        if readback != profile || parse_owned_profile(&readback)?.as_deref() != Some(&model) {
            return Err("codex_plugin_profile_readback_mismatch".to_string());
        }
        let receipt = CodexPluginReceipt {
            version: RECEIPT_VERSION,
            profile_path: path_string(&paths.profile),
            profile_sha256: sha256_hex(&profile),
            model: model.clone(),
            codex_version: detection
                .version
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            backup_present: existing_profile.is_some(),
        };
        let serialized = serde_json::to_string_pretty(&receipt)
            .map_err(|_| "codex_plugin_receipt_write_failed".to_string())?;
        write_secure_text(&paths.receipt, &serialized)?;
        let parsed = read_receipt(paths)?
            .ok_or_else(|| "codex_plugin_receipt_readback_missing".to_string())?;
        if parsed.profile_sha256 != receipt.profile_sha256 {
            return Err("codex_plugin_receipt_readback_mismatch".to_string());
        }
        Ok(())
    })();

    if let Err(error) = apply_result {
        restore_profile(&paths.profile, existing_profile.as_deref());
        restore_secret(&paths.credential, existing_token.as_ref());
        restore_secure(&paths.receipt, existing_receipt.as_deref());
        restore_secure(&paths.helper, existing_helper.as_deref());
        restore_secure(&paths.backup, existing_backup.as_deref());
        restore_secret(
            &paths.credential_backup,
            existing_credential_backup.as_ref(),
        );
        return Err(error);
    }

    let status = inspect_with(paths, detection);
    if status.state != "configured" {
        return Err(status
            .reason
            .clone()
            .unwrap_or_else(|| "codex_plugin_apply_failed".to_string()));
    }
    let mut changed_paths = vec![
        path_string(&paths.credential),
        path_string(&paths.profile),
        path_string(&paths.receipt),
    ];
    if cfg!(windows) {
        changed_paths.insert(1, path_string(&paths.helper));
    }
    Ok(CodexPluginOperationReceipt {
        operation: "apply".to_string(),
        status,
        changed_paths,
        backup_path: existing_profile
            .as_ref()
            .map(|_| path_string(&paths.backup)),
        config_valid: true,
        doctor_summary: None,
    })
}

fn verify_with(paths: &CodexPluginPaths) -> Result<CodexPluginOperationReceipt, String> {
    let detection = detect_codex();
    let status = inspect_with(paths, detection);
    if status.state != "configured" {
        return Err(status
            .reason
            .clone()
            .unwrap_or_else(|| "codex_plugin_not_configured".to_string()));
    }
    let profile =
        read_profile(paths)?.ok_or_else(|| "codex_plugin_profile_readback_missing".to_string())?;
    parse_owned_profile(&profile)?;
    let output = Command::new("codex")
        .env("CODEX_HOME", &paths.codex_home)
        .args([
            "--strict-config",
            "--profile",
            PROFILE_NAME,
            "doctor",
            "--json",
            "--summary",
        ])
        .output()
        .map_err(|_| "codex_plugin_doctor_failed".to_string())?;
    if !output.status.success() {
        return Err("codex_plugin_doctor_failed".to_string());
    }
    Ok(CodexPluginOperationReceipt {
        operation: "verify".to_string(),
        status,
        changed_paths: Vec::new(),
        backup_path: read_receipt(paths)?
            .filter(|receipt| receipt.backup_present)
            .map(|_| path_string(&paths.backup)),
        config_valid: true,
        doctor_summary: Some(
            "Codex strict-config doctor returned a valid redacted report.".to_string(),
        ),
    })
}

fn rollback_with(paths: &CodexPluginPaths) -> Result<CodexPluginOperationReceipt, String> {
    let detection = detect_codex();
    if let Some(profile) = read_profile(paths)? {
        parse_owned_profile(&profile)?;
    }
    let backup = read_secure_text(&paths.backup)?;
    let mut changed_paths = Vec::new();
    if let Some(backup) = backup {
        let model = parse_owned_profile(&backup)?
            .ok_or_else(|| "codex_plugin_profile_invalid".to_string())?;
        write_managed_profile(&paths.profile, &backup)?;
        if let Some(credential_backup) = secure_store(&paths.credential_backup).read()? {
            secure_store(&paths.credential).write(&credential_backup)?;
        } else {
            delete_secure_file(&paths.credential)?;
        }
        delete_secure_file(&paths.backup)?;
        delete_secure_file(&paths.credential_backup)?;
        let next_receipt = CodexPluginReceipt {
            version: RECEIPT_VERSION,
            profile_path: path_string(&paths.profile),
            profile_sha256: sha256_hex(&backup),
            model,
            codex_version: detection
                .version
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            backup_present: false,
        };
        write_secure_text(
            &paths.receipt,
            &serde_json::to_string_pretty(&next_receipt)
                .map_err(|_| "codex_plugin_receipt_write_failed".to_string())?,
        )?;
        changed_paths.extend([
            path_string(&paths.profile),
            path_string(&paths.backup),
            path_string(&paths.receipt),
        ]);
    } else {
        delete_managed_profile(&paths.profile)?;
        changed_paths.push(path_string(&paths.profile));
        for path in [
            &paths.credential,
            &paths.helper,
            &paths.receipt,
            &paths.backup,
            &paths.credential_backup,
        ] {
            delete_secure_file(path)?;
            changed_paths.push(path_string(path));
        }
        if paths.data_root.exists() {
            let _ = fs::remove_dir(&paths.data_root);
        }
    }
    Ok(CodexPluginOperationReceipt {
        operation: "rollback".to_string(),
        status: inspect_with(paths, detection),
        changed_paths,
        backup_path: None,
        config_valid: true,
        doctor_summary: None,
    })
}

#[tauri::command]
pub fn codex_plugin_inspect() -> Result<CodexPluginStatus, String> {
    let paths = resolve_paths()?;
    Ok(inspect(&paths))
}

#[tauri::command]
pub fn codex_plugin_preview(
    state: tauri::State<'_, crate::state::AppState>,
    model: String,
) -> Result<CodexPluginPreview, String> {
    validate_server_allowed_model(&state, model.trim())?;
    preview_with(&resolve_paths()?, &model)
}

#[tauri::command]
pub fn codex_plugin_apply(
    state: tauri::State<'_, crate::state::AppState>,
    model: String,
    credential: String,
) -> Result<CodexPluginOperationReceipt, String> {
    validate_server_allowed_model(&state, model.trim())?;
    apply_with(&resolve_paths()?, &model, &credential, detect_codex())
}

#[tauri::command]
pub fn codex_plugin_verify() -> Result<CodexPluginOperationReceipt, String> {
    verify_with(&resolve_paths()?)
}

#[tauri::command]
pub fn codex_plugin_rollback() -> Result<CodexPluginOperationReceipt, String> {
    rollback_with(&resolve_paths()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!("beefex-codex-plugin-{label}-{nonce}"));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn paths(directory: &TestDirectory) -> CodexPluginPaths {
        let codex = directory.0.join("codex-home");
        let app = directory.0.join("app-data");
        fs::create_dir_all(&codex).unwrap();
        fs::create_dir_all(&app).unwrap();
        CodexPluginPaths::new(codex, app)
    }

    fn supported() -> CodexDetection {
        CodexDetection {
            version: Some("0.146.0".to_string()),
            supported: true,
            reason: None,
        }
    }

    #[test]
    fn parses_supported_and_rejects_unrecognized_versions() {
        assert_eq!(
            parse_version("codex-cli 0.146.0"),
            Some(("0.146.0".to_string(), 0, 146, 0))
        );
        assert!(parse_version("codex development").is_none());
        let missing = detection_from_output(Err(()));
        assert_eq!(missing.reason.as_deref(), Some("codex_plugin_missing"));
        let unsupported = detection_from_output(Ok((true, "codex-cli 0.145.0".into())));
        assert_eq!(
            unsupported.reason.as_deref(),
            Some("codex_plugin_version_unsupported")
        );
    }

    #[test]
    fn profile_is_responses_only_and_never_contains_token() {
        let directory = TestDirectory::new("profile");
        let paths = paths(&directory);
        let profile = profile_text(&paths, "gpt-5.6-terra");
        let parsed: toml::Value = toml::from_str(&profile).unwrap();
        assert_eq!(parsed["model_provider"].as_str(), Some("beefapi"));
        assert_eq!(
            parsed["model_providers"]["beefapi"]["wire_api"].as_str(),
            Some("responses")
        );
        assert!(!profile.contains("never-print-this-token"));
        assert!(!profile.contains("experimental_bearer_token"));
        assert!(!profile.contains("group"));
        assert!(!profile.contains("route"));
    }

    #[test]
    fn apply_inspect_reapply_and_two_step_rollback() {
        let directory = TestDirectory::new("journey");
        let paths = paths(&directory);
        let first = apply_with(&paths, "gpt-5.6-terra", "separate-token-one", supported()).unwrap();
        assert_eq!(first.status.state, "configured");
        assert_eq!(
            first.status.configured_model.as_deref(),
            Some("gpt-5.6-terra")
        );

        let second = apply_with(&paths, "gpt-5.6-sol", "separate-token-two", supported()).unwrap();
        assert_eq!(
            second.status.configured_model.as_deref(),
            Some("gpt-5.6-sol")
        );
        assert!(paths.backup.exists());

        let restored = rollback_with(&paths).unwrap();
        assert_eq!(restored.status.state, "configured");
        assert_eq!(
            restored.status.configured_model.as_deref(),
            Some("gpt-5.6-terra")
        );
        assert_eq!(
            secure_store(&paths.credential)
                .read()
                .unwrap()
                .unwrap()
                .expose(),
            "separate-token-one"
        );

        let removed = rollback_with(&paths).unwrap();
        assert_eq!(removed.status.state, "ready");
        assert!(!paths.profile.exists());
        assert!(!paths.credential.exists());
        assert!(!paths.receipt.exists());
    }

    #[test]
    fn unowned_profile_is_a_hard_conflict_and_is_not_modified() {
        let directory = TestDirectory::new("conflict");
        let paths = paths(&directory);
        write_managed_profile(&paths.profile, "model = \"personal\"\n").unwrap();
        let before = read_profile(&paths).unwrap().unwrap();
        let error = apply_with(&paths, "gpt-5.6-terra", "separate-token", supported()).unwrap_err();
        assert_eq!(error, "codex_plugin_profile_conflict");
        assert_eq!(read_profile(&paths).unwrap().unwrap(), before);
        assert!(!paths.credential.exists());
    }

    #[test]
    fn missing_credential_or_tampered_profile_is_not_configured() {
        let directory = TestDirectory::new("tamper");
        let paths = paths(&directory);
        apply_with(&paths, "gpt-5.6-terra", "separate-token", supported()).unwrap();
        secure_store(&paths.credential).delete().unwrap();
        let missing = inspect_with(&paths, supported());
        assert_eq!(missing.state, "failed");
        assert_eq!(
            missing.reason.as_deref(),
            Some("codex_plugin_credential_missing")
        );

        write_secure_text(&paths.credential, "separate-token").unwrap();
        let mut profile = read_profile(&paths).unwrap().unwrap();
        profile.push_str("# tampered\n");
        write_managed_profile(&paths.profile, &profile).unwrap();
        let tampered = inspect_with(&paths, supported());
        assert_eq!(tampered.state, "failed");
        assert_eq!(
            tampered.reason.as_deref(),
            Some("codex_plugin_profile_readback_mismatch")
        );
    }

    #[test]
    fn invalid_inputs_are_rejected_without_writes() {
        let directory = TestDirectory::new("inputs");
        let paths = paths(&directory);
        assert_eq!(
            apply_with(&paths, "bad model", "separate-token", supported()).unwrap_err(),
            "codex_plugin_model_invalid"
        );
        assert_eq!(
            apply_with(&paths, "gpt-5.6-terra", "short", supported()).unwrap_err(),
            "codex_plugin_credential_invalid"
        );
        assert!(!paths.profile.exists());
        assert!(!paths.credential.exists());
    }

    #[test]
    fn debug_output_never_exposes_the_token() {
        let token = validate_token("never-print-this-token").unwrap();
        assert_eq!(format!("{token:?}"), "<redacted>");
    }

    #[cfg(unix)]
    #[test]
    fn profile_write_is_atomic_and_does_not_change_codex_home_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = TestDirectory::new("profile-permissions");
        let paths = paths(&directory);
        fs::set_permissions(&paths.codex_home, fs::Permissions::from_mode(0o755)).unwrap();

        write_managed_profile(&paths.profile, OWNERSHIP_MARKER).unwrap();

        assert_eq!(
            fs::metadata(&paths.codex_home)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(
            fs::metadata(&paths.profile).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(fs::read_dir(&paths.codex_home).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_relative_and_symlinked_roots_and_profile_write_failures() {
        use std::os::unix::fs::symlink;

        assert_eq!(
            validate_root(PathBuf::from("relative"), "codex_plugin_home").unwrap_err(),
            "codex_plugin_home_relative"
        );
        let directory = TestDirectory::new("unsafe-root");
        let actual = directory.0.join("actual");
        let linked = directory.0.join("linked");
        fs::create_dir_all(&actual).unwrap();
        symlink(&actual, &linked).unwrap();
        assert_eq!(
            validate_root(linked, "codex_plugin_home").unwrap_err(),
            "codex_plugin_home_unsafe"
        );

        let blocked = directory.0.join("blocked");
        fs::write(&blocked, "not a directory").unwrap();
        let target = blocked.join(PROFILE_FILE);
        assert_eq!(
            write_managed_profile(&target, OWNERSHIP_MARKER).unwrap_err(),
            "codex_plugin_profile_write_failed"
        );
        assert_eq!(fs::read_to_string(&blocked).unwrap(), "not a directory");
    }
}
