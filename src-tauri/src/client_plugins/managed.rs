use crate::{
    app_paths,
    beefapi::{
        credential_store::{CredentialStore, FileCredentialStore, SecretCredential},
        types::AccountPhase,
    },
    client_plugins::codex,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tauri::{AppHandle, Manager, State};
use toml_edit::{value, DocumentMut, Item, Table};

const RECEIPT_VERSION: u8 = 1;
const MAX_CONFIG_BYTES: u64 = 2 * 1024 * 1024;
const GROK_MODEL: &str = "grok-4.6";
const IMAGE2_INSTALLER_UNIX: &str = "beefapi-codex-image2.sh";
const IMAGE2_INSTALLER_WINDOWS: &str = "beefapi-codex-image2.ps1";

fn image2_installer_name() -> &'static str {
    if cfg!(windows) {
        IMAGE2_INSTALLER_WINDOWS
    } else {
        IMAGE2_INSTALLER_UNIX
    }
}

#[derive(Clone, Debug)]
struct ManagedPaths {
    claude_credential: PathBuf,
    claude_helper: PathBuf,
    receipt: PathBuf,
    backup_root: PathBuf,
    claude_code: PathBuf,
    claude_desktop: PathBuf,
    claude_desktop_3p: PathBuf,
    claude_profile: PathBuf,
    claude_profiles_meta: PathBuf,
    grok: PathBuf,
    image2_cli: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedClientItem {
    pub(crate) id: String,
    detected: bool,
    pub(crate) configured: bool,
    launch_command: String,
    reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedClientsStatus {
    state: String,
    pub(crate) clients: Vec<ManagedClientItem>,
    reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedClientsReceipt {
    operation: String,
    pub(crate) status: ManagedClientsStatus,
    changed_paths: Vec<String>,
    checks: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FileReceipt {
    path: String,
    managed_sha256: String,
    original_existed: bool,
    backup_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ManagedReceipt {
    version: u8,
    files: Vec<FileReceipt>,
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn home_dir() -> Result<PathBuf, String> {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .ok_or_else(|| "managed_clients_home_unavailable".into())
}

fn resolve_paths() -> Result<ManagedPaths, String> {
    let home = home_dir()?;
    let data_root = app_paths::app_data_dir()
        .ok_or_else(|| "managed_clients_data_unavailable".to_string())?
        .join("client-plugins")
        .join("managed-clients");
    let codex_home = env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let claude_base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(windows) {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
    } else {
        home.join(".config")
    };
    Ok(ManagedPaths {
        claude_credential: data_root.join("credentials").join("claude.credential"),
        claude_helper: data_root.join(if cfg!(windows) {
            "claude-credential.ps1"
        } else {
            "claude-credential.sh"
        }),
        receipt: data_root.join("receipt.json"),
        backup_root: data_root.join("backups"),
        claude_code: home.join(".claude/settings.json"),
        claude_desktop: claude_base.join("Claude/claude_desktop_config.json"),
        claude_desktop_3p: claude_base.join("Claude-3p/claude_desktop_config.json"),
        claude_profile: claude_base.join("Claude-3p/configLibrary/beefapi-managed.json"),
        claude_profiles_meta: claude_base.join("Claude-3p/configLibrary/_meta.json"),
        grok: home.join(".grok/config.toml"),
        image2_cli: codex_home.join("skills/beefapi-image2/scripts/beefapi-image2.mjs"),
    })
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let meta = match fs::symlink_metadata(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("managed_clients_read_failed".into()),
    };
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > MAX_CONFIG_BYTES {
        return Err("managed_clients_path_unsafe".into());
    }
    fs::read(path)
        .map(Some)
        .map_err(|_| "managed_clients_read_failed".into())
}

fn atomic_write(path: &Path, bytes: &[u8], private: bool) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "managed_clients_write_failed".to_string())?;
    fs::create_dir_all(parent).map_err(|_| "managed_clients_write_failed".to_string())?;
    let tmp = parent.join(format!(".beefex-managed.{}.tmp", std::process::id()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&tmp)
            .map_err(|_| "managed_clients_write_failed".to_string())?;
        file.write_all(bytes)
            .map_err(|_| "managed_clients_write_failed".to_string())?;
        file.sync_all()
            .map_err(|_| "managed_clients_write_failed".to_string())?;
        drop(file);
        fs::rename(&tmp, path).map_err(|_| "managed_clients_write_failed".to_string())?;
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .map_err(|_| "managed_clients_write_failed".to_string())?;
        }
        Ok(())
    })();
    let _ = fs::remove_file(tmp);
    result
}

fn json_object(bytes: Option<&[u8]>) -> Result<Map<String, Value>, String> {
    match bytes {
        None => Ok(Map::new()),
        Some(bytes) => serde_json::from_slice::<Value>(bytes)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .ok_or_else(|| "managed_clients_config_invalid".into()),
    }
}

fn claude_helper_text(path: &Path) -> String {
    if cfg!(windows) {
        format!("powershell.exe -NoLogo -NoProfile -NonInteractive -Command \"[Console]::Out.Write(([IO.File]::ReadAllText('{}')).Trim())\"", path_string(path).replace('\'', "''"))
    } else {
        format!("/bin/cat '{}'", path_string(path).replace('\'', "'\\''"))
    }
}

fn anthropic_base(origin: &str) -> &str {
    origin
        .trim_end_matches('/')
        .strip_suffix("/v1")
        .unwrap_or_else(|| origin.trim_end_matches('/'))
}

fn merge_claude_code(
    original: Option<&[u8]>,
    paths: &ManagedPaths,
    origin: &str,
) -> Result<Vec<u8>, String> {
    let mut root = json_object(original)?;
    let env = root
        .entry("env")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "managed_clients_config_conflict".to_string())?;
    env.insert(
        "ANTHROPIC_BASE_URL".into(),
        Value::String(anthropic_base(origin).to_string()),
    );
    env.remove("ANTHROPIC_AUTH_TOKEN");
    root.insert(
        "apiKeyHelper".into(),
        Value::String(claude_helper_text(&paths.claude_credential)),
    );
    serde_json::to_vec_pretty(&root).map_err(|_| "managed_clients_config_invalid".into())
}

fn merge_json_fields(original: Option<&[u8]>, fields: &[(&str, Value)]) -> Result<Vec<u8>, String> {
    let mut root = json_object(original)?;
    for (key, value) in fields {
        root.insert((*key).into(), value.clone());
    }
    serde_json::to_vec_pretty(&root).map_err(|_| "managed_clients_config_invalid".into())
}

fn claude_profile(origin: &str, credential: &SecretCredential) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(&json!({
        "coworkEgressAllowedHosts": ["*"],
        "disableDeploymentModeChooser": true,
        "inferenceGatewayApiKey": credential.expose(),
        "inferenceGatewayAuthScheme": "bearer",
        "inferenceGatewayBaseUrl": anthropic_base(origin),
        "inferenceProvider": "gateway"
    }))
    .map_err(|_| "managed_clients_config_invalid".into())
}

fn merge_profiles_meta(original: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut root = json_object(original)?;
    let profiles = root
        .entry("profiles")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| "managed_clients_config_conflict".to_string())?;
    profiles.retain(|v| v.get("id").and_then(Value::as_str) != Some("beefapi-managed"));
    profiles.push(json!({"id":"beefapi-managed","name":"BeefAPI (managed by Beefex)","file":"beefapi-managed.json"}));
    root.insert(
        "lastActiveProfileId".into(),
        Value::String("beefapi-managed".into()),
    );
    serde_json::to_vec_pretty(&root).map_err(|_| "managed_clients_config_invalid".into())
}

fn merge_grok(
    original: Option<&[u8]>,
    origin: &str,
    credential: &SecretCredential,
) -> Result<Vec<u8>, String> {
    let source = original
        .map(|v| std::str::from_utf8(v).map_err(|_| "managed_clients_config_invalid".to_string()))
        .transpose()?
        .unwrap_or("");
    let mut doc = if source.trim().is_empty() {
        DocumentMut::new()
    } else {
        source
            .parse::<DocumentMut>()
            .map_err(|_| "managed_clients_config_invalid".to_string())?
    };
    if doc.get("model").is_none() {
        doc["model"] = Item::Table(Table::new());
    }
    if !doc["model"].is_table() {
        return Err("managed_clients_config_conflict".into());
    }
    let mut provider = Table::new();
    provider["model"] = value(GROK_MODEL);
    provider["base_url"] = value(origin.trim_end_matches('/'));
    provider["name"] = value("Grok 4.6 via BeefAPI");
    provider["api_key"] = value(credential.expose());
    provider["api_backend"] = value("responses");
    doc["model"]
        .as_table_mut()
        .unwrap()
        .insert("beefapi-grok", Item::Table(provider));
    if doc.get("models").is_none() {
        doc["models"] = Item::Table(Table::new());
    }
    if !doc["models"].is_table() {
        return Err("managed_clients_config_conflict".into());
    }
    doc["models"]["default"] = value("beefapi-grok");
    Ok(doc.to_string().into_bytes())
}

fn read_receipt(paths: &ManagedPaths) -> Result<Option<ManagedReceipt>, String> {
    read_bounded(&paths.receipt)?
        .map(|v| {
            serde_json::from_slice(&v).map_err(|_| "managed_clients_receipt_invalid".to_string())
        })
        .transpose()
}

fn write_managed_file(
    paths: &ManagedPaths,
    path: &Path,
    bytes: &[u8],
    private: bool,
    previous: Option<&ManagedReceipt>,
) -> Result<FileReceipt, String> {
    let current = read_bounded(path)?;
    let old = previous.and_then(|r| r.files.iter().find(|f| f.path == path_string(path)));
    if let Some(old) = old {
        if current.as_deref().map(sha256).as_deref() != Some(old.managed_sha256.as_str()) {
            return Err("managed_clients_config_changed".into());
        }
    }
    let slug = format!("{}.backup", sha256(path_string(path).as_bytes()));
    let backup = old
        .map(|v| PathBuf::from(&v.backup_path))
        .unwrap_or_else(|| paths.backup_root.join(slug));
    let original_existed = old.map(|v| v.original_existed).unwrap_or(current.is_some());
    if old.is_none() {
        if let Some(original) = current.as_deref() {
            atomic_write(&backup, original, true)?;
        }
    }
    atomic_write(path, bytes, private)?;
    let readback =
        read_bounded(path)?.ok_or_else(|| "managed_clients_readback_failed".to_string())?;
    if readback != bytes {
        return Err("managed_clients_readback_failed".into());
    }
    Ok(FileReceipt {
        path: path_string(path),
        managed_sha256: sha256(bytes),
        original_existed,
        backup_path: path_string(&backup),
    })
}

fn configure_direct_clients(
    paths: &ManagedPaths,
    origin: &str,
    claude: &SecretCredential,
    grok: &SecretCredential,
) -> Result<Vec<FileReceipt>, String> {
    let previous = read_receipt(paths)?;
    let credential_store = FileCredentialStore::new(paths.claude_credential.clone());
    let previous_credential = credential_store.read()?;
    let previous_helper = read_bounded(&paths.claude_helper)?;
    let previous_receipt = read_bounded(&paths.receipt)?;
    let specs = vec![
        (
            paths.claude_code.clone(),
            merge_claude_code(read_bounded(&paths.claude_code)?.as_deref(), paths, origin)?,
            true,
        ),
        (
            paths.claude_desktop.clone(),
            merge_json_fields(
                read_bounded(&paths.claude_desktop)?.as_deref(),
                &[("deploymentMode", json!("3p"))],
            )?,
            false,
        ),
        (
            paths.claude_desktop_3p.clone(),
            merge_json_fields(
                read_bounded(&paths.claude_desktop_3p)?.as_deref(),
                &[("deploymentMode", json!("3p"))],
            )?,
            false,
        ),
        (
            paths.claude_profile.clone(),
            claude_profile(origin, claude)?,
            true,
        ),
        (
            paths.claude_profiles_meta.clone(),
            merge_profiles_meta(read_bounded(&paths.claude_profiles_meta)?.as_deref())?,
            false,
        ),
        (
            paths.grok.clone(),
            merge_grok(read_bounded(&paths.grok)?.as_deref(), origin, grok)?,
            true,
        ),
    ];
    let snapshots = specs
        .iter()
        .map(|(path, _, private)| Ok((path.clone(), read_bounded(path)?, *private)))
        .collect::<Result<Vec<_>, String>>()?;
    let result = (|| {
        credential_store.write(claude)?;
        if cfg!(windows) {
            atomic_write(&paths.claude_helper, b"$ErrorActionPreference='Stop'\n[Console]::Out.Write(([IO.File]::ReadAllText($args[0])).Trim())\n", true)?;
        }
        let mut files = Vec::new();
        for (path, bytes, private) in specs {
            files.push(write_managed_file(
                paths,
                &path,
                &bytes,
                private,
                previous.as_ref(),
            )?);
        }
        atomic_write(
            &paths.receipt,
            &serde_json::to_vec_pretty(&ManagedReceipt {
                version: RECEIPT_VERSION,
                files: files.clone(),
            })
            .map_err(|_| "managed_clients_receipt_invalid".to_string())?,
            true,
        )?;
        Ok(files)
    })();
    if result.is_err() {
        for (path, snapshot, private) in snapshots.iter().rev() {
            let _ = restore_snapshot(path, snapshot.as_deref(), *private);
        }
        match previous_credential.as_ref() {
            Some(value) => {
                let _ = credential_store.write(value);
            }
            None => {
                let _ = credential_store.delete();
            }
        }
        let _ = restore_snapshot(&paths.claude_helper, previous_helper.as_deref(), true);
        let _ = restore_snapshot(&paths.receipt, previous_receipt.as_deref(), true);
    }
    result
}

fn restore_snapshot(path: &Path, snapshot: Option<&[u8]>, private: bool) -> Result<(), String> {
    if let Some(bytes) = snapshot {
        atomic_write(path, bytes, private)
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("managed_clients_rollback_failed".into()),
        }
    }
}

fn command_detected(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn inspect_with(paths: &ManagedPaths) -> Result<ManagedClientsStatus, String> {
    let receipt = read_receipt(paths)?;
    let files_ok = receipt.as_ref().is_some_and(|r| {
        r.version == RECEIPT_VERSION
            && r.files.iter().all(|f| {
                read_bounded(Path::new(&f.path))
                    .ok()
                    .flatten()
                    .is_some_and(|v| sha256(&v) == f.managed_sha256)
            })
    });
    let codex_status = codex::codex_plugin_inspect()?;
    let clients = vec![
        ManagedClientItem {
            id: "codex".into(),
            detected: codex_status.supported,
            configured: codex_status.state == "configured",
            launch_command: "codex".into(),
            reason: codex_status.reason,
        },
        ManagedClientItem {
            id: "image2".into(),
            detected: command_detected("node"),
            configured: paths.image2_cli.is_file(),
            launch_command: "beefapi-image2 doctor".into(),
            reason: None,
        },
        ManagedClientItem {
            id: "claude-code".into(),
            detected: command_detected("claude"),
            configured: files_ok,
            launch_command: "claude".into(),
            reason: None,
        },
        ManagedClientItem {
            id: "claude-desktop".into(),
            detected: cfg!(target_os = "macos") || cfg!(windows),
            configured: files_ok,
            launch_command: "Claude".into(),
            reason: None,
        },
        ManagedClientItem {
            id: "grok".into(),
            detected: command_detected("grok"),
            configured: files_ok,
            launch_command: "grok".into(),
            reason: None,
        },
    ];
    let configured = clients.iter().filter(|v| v.configured).count();
    Ok(ManagedClientsStatus {
        state: if configured == clients.len() {
            "configured"
        } else if configured > 0 {
            "partial"
        } else {
            "ready"
        }
        .into(),
        clients,
        reason: None,
    })
}

fn image2_installer(app: &AppHandle) -> Result<PathBuf, String> {
    let installer = image2_installer_name();
    let packaged = app
        .path()
        .resource_dir()
        .ok()
        .map(|p| p.join("client-plugins").join(installer));
    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/client-plugins")
        .join(installer);
    packaged
        .filter(|p| p.is_file())
        .or_else(|| development.is_file().then_some(development))
        .ok_or_else(|| "managed_clients_image2_installer_missing".into())
}

fn install_image2(
    app: &AppHandle,
    origin: &str,
    credential: &SecretCredential,
) -> Result<(), String> {
    let installer = image2_installer(app)?;
    let mut command = if cfg!(windows) {
        let mut c = Command::new("powershell.exe");
        c.args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ]);
        c
    } else {
        Command::new("/bin/sh")
    };
    let status = command
        .arg(installer)
        .arg("--skip-check")
        .env("BEEFAPI_IMAGE2_API_KEY", credential.expose())
        .env("BEEFAPI_IMAGE2_BASE_URL", origin.trim_end_matches('/'))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| "managed_clients_image2_install_failed".to_string())?;
    if !status.success() {
        return Err("managed_clients_image2_install_failed".into());
    }
    Ok(())
}

#[tauri::command]
pub fn managed_clients_inspect() -> Result<ManagedClientsStatus, String> {
    inspect_with(&resolve_paths()?)
}

#[tauri::command]
pub async fn managed_clients_apply(
    app: AppHandle,
    state: State<'_, crate::state::AppState>,
    model: String,
) -> Result<ManagedClientsReceipt, String> {
    apply_from_state(&app, &state, model).await
}

pub(crate) async fn apply_from_state(
    app: &AppHandle,
    state: &crate::state::AppState,
    model: String,
) -> Result<ManagedClientsReceipt, String> {
    if state.beefapi_account.state().phase != AccountPhase::SignedIn {
        return Err("managed_clients_sign_in_required".into());
    }
    codex::validate_server_allowed_model(&state, &model)?;
    let credentials = state.beefapi_account.ensure_client_credentials().await?;
    if credentials.gpt.group != "gpt-pro"
        || credentials.claude.group != "claude max"
        || credentials.grok.group != "grok"
    {
        return Err("managed_clients_group_contract_invalid".into());
    }
    let paths = resolve_paths()?;
    let had_direct_configuration = read_receipt(&paths)?.is_some();
    let had_codex_configuration = codex::codex_plugin_inspect()?.state == "configured";
    codex::apply_managed(
        &codex::resolve_paths()?,
        &model,
        &credentials.gpt.credential,
        codex::detect_codex(),
    )
    .await?;
    let files = match configure_direct_clients(
        &paths,
        &credentials.origin,
        &credentials.claude.credential,
        &credentials.grok.credential,
    ) {
        Ok(files) => files,
        Err(error) => {
            if !had_codex_configuration {
                let _ = codex::codex_plugin_rollback();
            }
            return Err(error);
        }
    };
    if let Err(error) = install_image2(app, &credentials.origin, &credentials.gpt.credential) {
        if !had_direct_configuration {
            let _ = restore_direct(&paths);
        }
        if !had_codex_configuration {
            let _ = codex::codex_plugin_rollback();
        }
        return Err(error);
    }
    Ok(ManagedClientsReceipt {
        operation: "apply".into(),
        status: inspect_with(&paths)?,
        changed_paths: files.into_iter().map(|f| f.path).collect(),
        checks: vec![
            "managed keys: gpt-pro, claude max, grok".into(),
            "config readback".into(),
        ],
    })
}

#[tauri::command]
pub fn managed_clients_verify() -> Result<ManagedClientsReceipt, String> {
    let paths = resolve_paths()?;
    let status = inspect_with(&paths)?;
    if status.state != "configured" {
        return Err("managed_clients_not_fully_configured".into());
    }
    let codex_check = codex::codex_plugin_verify()?;
    let image = Command::new("node")
        .arg(&paths.image2_cli)
        .args(["doctor", "--offline"])
        .output()
        .map_err(|_| "managed_clients_image2_doctor_failed".to_string())?;
    if !image.status.success() {
        return Err("managed_clients_image2_doctor_failed".into());
    }
    if !command_detected("claude") {
        return Err("managed_clients_claude_missing".into());
    }
    if !command_detected("grok") {
        return Err("managed_clients_grok_missing".into());
    }
    Ok(ManagedClientsReceipt {
        operation: "verify".into(),
        status,
        changed_paths: vec![],
        checks: vec![
            codex_check.doctor_summary.unwrap_or_default(),
            "Image2 offline doctor".into(),
            "Claude version and config hashes".into(),
            "Grok version and config hashes".into(),
        ],
    })
}

fn restore_direct(paths: &ManagedPaths) -> Result<Vec<String>, String> {
    let receipt =
        read_receipt(paths)?.ok_or_else(|| "managed_clients_not_configured".to_string())?;
    for file in &receipt.files {
        let path = Path::new(&file.path);
        let current =
            read_bounded(path)?.ok_or_else(|| "managed_clients_config_changed".to_string())?;
        if sha256(&current) != file.managed_sha256 {
            return Err("managed_clients_config_changed".into());
        }
    }
    for file in &receipt.files {
        let path = Path::new(&file.path);
        if file.original_existed {
            let backup = read_bounded(Path::new(&file.backup_path))?
                .ok_or_else(|| "managed_clients_backup_missing".to_string())?;
            atomic_write(path, &backup, false)?;
        } else {
            fs::remove_file(path).map_err(|_| "managed_clients_rollback_failed".to_string())?;
        }
    }
    FileCredentialStore::new(paths.claude_credential.clone()).delete()?;
    let _ = fs::remove_file(&paths.claude_helper);
    let _ = fs::remove_file(&paths.receipt);
    for file in &receipt.files {
        let _ = fs::remove_file(&file.backup_path);
    }
    Ok(receipt.files.into_iter().map(|f| f.path).collect())
}

#[tauri::command]
pub fn managed_clients_rollback() -> Result<ManagedClientsReceipt, String> {
    let paths = resolve_paths()?;
    if paths.image2_cli.is_file() {
        let _ = Command::new("node")
            .arg(&paths.image2_cli)
            .args(["uninstall", "--purge-credentials"])
            .status();
    }
    let mut changed = restore_direct(&paths)?;
    let codex_receipt = codex::codex_plugin_rollback()?;
    changed.extend(codex_receipt.changed_paths);
    Ok(ManagedClientsReceipt {
        operation: "rollback".into(),
        status: inspect_with(&paths)?,
        changed_paths: changed,
        checks: vec!["owned files restored".into()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> ManagedPaths {
        let root = env::temp_dir().join(format!("beefex-managed-clients-{}", uuid::Uuid::new_v4()));
        ManagedPaths {
            claude_credential: root.join("data/claude"),
            claude_helper: root.join("data/helper"),
            receipt: root.join("data/receipt.json"),
            backup_root: root.join("data/backups"),
            claude_code: root.join(".claude/settings.json"),
            claude_desktop: root.join("Claude/config.json"),
            claude_desktop_3p: root.join("Claude-3p/config.json"),
            claude_profile: root.join("Claude-3p/profiles/beefapi-managed.json"),
            claude_profiles_meta: root.join("Claude-3p/profiles/_meta.json"),
            grok: root.join(".grok/config.toml"),
            image2_cli: root.join(".codex/skills/beefapi-image2/scripts/beefapi-image2.mjs"),
        }
    }

    #[test]
    fn image2_installer_matches_host_shell() {
        if cfg!(windows) {
            assert_eq!(image2_installer_name(), "beefapi-codex-image2.ps1");
        } else {
            assert_eq!(image2_installer_name(), "beefapi-codex-image2.sh");
        }
    }

    #[test]
    fn direct_configs_preserve_unrelated_values_and_use_exact_groups_without_receipt_secrets() {
        let paths = fixture();
        atomic_write(
            &paths.claude_code,
            br#"{"theme":"dark","env":{"KEEP":"yes"}}"#,
            false,
        )
        .unwrap();
        atomic_write(&paths.grok, b"theme = \"dark\"\n", false).unwrap();
        let claude = SecretCredential::new("sk-claude-secret".into());
        let grok = SecretCredential::new("sk-grok-secret".into());
        configure_direct_clients(&paths, "https://beefapi.com/v1", &claude, &grok).unwrap();
        let settings: Value =
            serde_json::from_slice(&fs::read(&paths.claude_code).unwrap()).unwrap();
        assert_eq!(settings["theme"], "dark");
        assert_eq!(settings["env"]["KEEP"], "yes");
        assert_eq!(settings["env"]["ANTHROPIC_BASE_URL"], "https://beefapi.com");
        assert!(settings.get("ANTHROPIC_AUTH_TOKEN").is_none());
        let grok_config = fs::read_to_string(&paths.grok).unwrap();
        assert!(grok_config.contains("grok-4.6"));
        assert!(grok_config.contains("responses"));
        let receipt = fs::read_to_string(&paths.receipt).unwrap();
        assert!(!receipt.contains("sk-claude-secret"));
        assert!(!receipt.contains("sk-grok-secret"));
    }

    #[test]
    fn rollback_restores_exact_preimages_and_refuses_changed_managed_file() {
        let paths = fixture();
        atomic_write(&paths.claude_code, br#"{"theme":"original"}"#, false).unwrap();
        let original = fs::read(&paths.claude_code).unwrap();
        configure_direct_clients(
            &paths,
            "https://beefapi.com/v1",
            &SecretCredential::new("claude".into()),
            &SecretCredential::new("grok".into()),
        )
        .unwrap();
        atomic_write(&paths.grok, b"user changed", false).unwrap();
        assert_eq!(
            restore_direct(&paths).unwrap_err(),
            "managed_clients_config_changed"
        );
        let managed = read_receipt(&paths).unwrap().unwrap();
        let grok_receipt = managed
            .files
            .iter()
            .find(|f| f.path == path_string(&paths.grok))
            .unwrap();
        let expected = merge_grok(
            None,
            "https://beefapi.com/v1",
            &SecretCredential::new("grok".into()),
        )
        .unwrap();
        assert_eq!(sha256(&expected), grok_receipt.managed_sha256);
        atomic_write(&paths.grok, &expected, true).unwrap();
        restore_direct(&paths).unwrap();
        assert_eq!(fs::read(&paths.claude_code).unwrap(), original);
        assert!(!paths.grok.exists());
    }

    #[test]
    fn failed_multi_client_write_restores_prior_files_and_credentials() {
        let paths = fixture();
        atomic_write(&paths.claude_code, br#"{"theme":"original"}"#, false).unwrap();
        let original = fs::read(&paths.claude_code).unwrap();
        atomic_write(
            paths.grok.parent().unwrap(),
            b"blocks grok directory",
            false,
        )
        .unwrap();
        let error = configure_direct_clients(
            &paths,
            "https://beefapi.com/v1",
            &SecretCredential::new("claude".into()),
            &SecretCredential::new("grok".into()),
        )
        .unwrap_err();
        assert!(matches!(
            error.as_str(),
            "managed_clients_read_failed" | "managed_clients_write_failed"
        ));
        assert_eq!(fs::read(&paths.claude_code).unwrap(), original);
        assert!(FileCredentialStore::new(paths.claude_credential.clone())
            .read()
            .unwrap()
            .is_none());
        assert!(!paths.receipt.exists());
    }
}
