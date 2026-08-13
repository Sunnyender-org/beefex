use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use tauri::{AppHandle, Manager};

const TRUST_FILE_NAME: &str = "trust.json";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PiProjectTrustPreview {
    pub requested_path: String,
    pub trust_path: String,
    pub is_git_repository: bool,
    pub decision: String,
    pub inherited_from: Option<String>,
    pub resources: Vec<&'static str>,
}

fn pi_agent_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| format!("app_data_dir unavailable: {error}"))?
        .join("pi-runtime")
        .join("agent"))
}

fn trust_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(pi_agent_dir(app)?.join(TRUST_FILE_NAME))
}

fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_dir() {
        return Err("项目文件夹不存在或不是目录".to_string());
    }
    path.canonicalize()
        .map_err(|error| format!("无法解析项目文件夹: {error}"))
}

pub fn resolve_project_trust_root(path: &Path) -> Result<(PathBuf, bool), String> {
    let requested = canonical_directory(path)?;
    let output = Command::new("git")
        .arg("-C")
        .arg(&requested)
        .args(["rev-parse", "--show-toplevel"])
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let root =
                String::from_utf8(output.stdout).map_err(|_| "Git 返回了无效路径".to_string())?;
            let root = canonical_directory(Path::new(root.trim()))?;
            return Ok((root, true));
        }
    }

    Ok((requested, false))
}

type TrustMap = BTreeMap<String, Option<bool>>;

fn read_trust_map(path: &Path) -> Result<TrustMap, String> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("无法检查 Pi 项目信任文件: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Pi 项目信任文件必须是普通文件".to_string());
    }
    let content =
        fs::read_to_string(path).map_err(|error| format!("无法读取 Pi 项目信任文件: {error}"))?;
    serde_json::from_str(&content).map_err(|error| format!("Pi 项目信任文件格式无效: {error}"))
}

fn decision_for<'a>(trust_map: &'a TrustMap, root: &Path) -> Option<(&'a str, bool)> {
    trust_map
        .iter()
        .filter_map(|(candidate, trusted)| {
            let trusted = (*trusted)?;
            let candidate_path = Path::new(candidate);
            root.starts_with(candidate_path).then_some((
                candidate.as_str(),
                trusted,
                candidate_path.components().count(),
            ))
        })
        .max_by_key(|(_, _, depth)| *depth)
        .map(|(candidate, trusted, _)| (candidate, trusted))
}

#[cfg(unix)]
fn secure_dir(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path).map_err(|error| format!("无法创建 Pi 配置目录: {error}"))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("无法保护 Pi 配置目录: {error}"))
}

#[cfg(not(unix))]
fn secure_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| format!("无法创建 Pi 配置目录: {error}"))
}

fn write_trust_map(path: &Path, trust_map: &TrustMap) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Pi 项目信任路径无父目录".to_string())?;
    secure_dir(parent)?;
    let content = serde_json::to_string_pretty(trust_map)
        .map_err(|error| format!("无法序列化 Pi 项目信任: {error}"))?;
    let temp = parent.join(format!(".{TRUST_FILE_NAME}.tmp.{}", uuid::Uuid::new_v4()));

    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp)
        .map_err(|error| format!("无法创建 Pi 项目信任临时文件: {error}"))?;
    file.write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("无法写入 Pi 项目信任: {error}"))?;
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        format!("无法原子保存 Pi 项目信任: {error}")
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("无法保护 Pi 项目信任文件: {error}"))?;
    }
    let readback = read_trust_map(path)?;
    if &readback != trust_map {
        return Err("Pi 项目信任保存后校验失败".to_string());
    }
    Ok(())
}

pub fn preview_project_trust(
    app: &AppHandle,
    requested_path: &Path,
) -> Result<PiProjectTrustPreview, String> {
    let requested = canonical_directory(requested_path)?;
    let (root, is_git_repository) = resolve_project_trust_root(&requested)?;
    let trust_map = read_trust_map(&trust_file_path(app)?)?;
    let existing = decision_for(&trust_map, &root);
    Ok(PiProjectTrustPreview {
        requested_path: requested.to_string_lossy().into_owned(),
        trust_path: root.to_string_lossy().into_owned(),
        is_git_repository,
        decision: existing
            .map(|(_, trusted)| if trusted { "trusted" } else { "untrusted" })
            .unwrap_or("unknown")
            .to_string(),
        inherited_from: existing.map(|(path, _)| path.to_string()),
        resources: vec![
            ".pi/settings.json",
            ".pi/extensions",
            ".pi/skills",
            ".pi/prompts",
            ".pi/themes",
            ".pi/SYSTEM.md",
            ".pi/APPEND_SYSTEM.md",
            ".agents/skills",
            "project packages",
        ],
    })
}

pub fn set_project_trust(
    app: &AppHandle,
    requested_path: &Path,
    trusted: Option<bool>,
) -> Result<PiProjectTrustPreview, String> {
    let (root, _) = resolve_project_trust_root(requested_path)?;
    let path = trust_file_path(app)?;
    let mut trust_map = read_trust_map(&path)?;
    let key = root.to_string_lossy().into_owned();
    match trusted {
        Some(value) => {
            trust_map.insert(key, Some(value));
        }
        None => {
            trust_map.remove(&key);
        }
    }
    write_trust_map(&path, &trust_map)?;
    preview_project_trust(app, requested_path)
}

pub fn require_project_trust(app: &AppHandle, requested_path: &Path) -> Result<(), String> {
    let preview = preview_project_trust(app, requested_path)?;
    if preview.decision == "trusted" {
        Ok(())
    } else {
        Err(format!("pi_project_trust_required:{}", preview.trust_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_parent_decision_wins() {
        let mut map = BTreeMap::new();
        map.insert("/tmp/work".to_string(), Some(true));
        map.insert("/tmp/work/blocked".to_string(), Some(false));
        map.insert("/tmp/work/ignored".to_string(), None);
        assert_eq!(
            decision_for(&map, Path::new("/tmp/work/app")),
            Some(("/tmp/work", true))
        );
        assert_eq!(
            decision_for(&map, Path::new("/tmp/work/blocked/app")),
            Some(("/tmp/work/blocked", false))
        );
    }

    #[test]
    fn trust_file_roundtrip_is_sorted_and_owner_only() {
        let root = std::env::temp_dir().join(format!("beefex-pi-trust-{}", uuid::Uuid::new_v4()));
        let path = root.join(TRUST_FILE_NAME);
        let mut map = BTreeMap::new();
        map.insert("/z".to_string(), Some(false));
        map.insert("/a".to_string(), Some(true));
        write_trust_map(&path, &map).unwrap();
        assert_eq!(read_trust_map(&path).unwrap(), map);
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.find("/a").unwrap() < content.find("/z").unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_or_symlink_trust_file_fails_closed() {
        let root =
            std::env::temp_dir().join(format!("beefex-pi-trust-bad-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join(TRUST_FILE_NAME);
        fs::write(&path, "[]").unwrap();
        assert!(read_trust_map(&path).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            fs::remove_file(&path).unwrap();
            let target = root.join("target.json");
            fs::write(&target, "{}").unwrap();
            symlink(&target, &path).unwrap();
            assert!(read_trust_map(&path).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nested_git_directory_resolves_to_canonical_repository_root() {
        let root =
            std::env::temp_dir().join(format!("beefex-pi-trust-git-{}", uuid::Uuid::new_v4()));
        let nested = root.join("apps/desktop");
        fs::create_dir_all(&nested).unwrap();
        let output = Command::new("git").arg("init").arg(&root).output().unwrap();
        assert!(output.status.success());
        let (resolved, is_git) = resolve_project_trust_root(&nested).unwrap();
        assert!(is_git);
        assert_eq!(resolved, root.canonicalize().unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}
