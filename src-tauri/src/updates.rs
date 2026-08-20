use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State};
#[cfg(any(target_os = "macos", test))]
use uuid::Uuid;

use crate::api::with_standard_request_timeout;
use crate::state::AppState;

const R2_LATEST_BASE: &str =
    "https://pub-e540a6ea6d6e4af19d7f5fc4d1f07c47.r2.dev/beefex/releases/latest";
const GITHUB_RELEASES_URL: &str = "https://api.github.com/repos/Sunnyender-org/beefex/releases";
const PUBLIC_DOWNLOAD_PAGE: &str = "https://beefapi.com/download";
const UPDATER_SCHEMA: &str = "beefex.updater.v1";
const ARTIFACT_SCHEMA: &str = "beefex.alpha-artifact.v1";
const PRODUCT_NAME: &str = "Beefex";
const PRODUCT_IDENTIFIER: &str = "com.beefapi.beefex";
const USER_DATA_MARKER: &str = "com.beefapi.beefex";

/// 检查当前 Alpha 线是否有更新。
///
/// 主通道是 R2 `beefex-updater.json`（Beefex 自有合同，不是 Electron latest.yml）。
/// 该对象尚未发布时，回退到 GitHub `beefex.alpha-artifact.v1` + R2 `SHA256SUMS.txt`，
/// 安装包永远走下载页同名的 R2 latest 对象。旧的 `latest.json` / `latest.yml` 会被拒绝。
#[tauri::command]
pub(crate) async fn check_github_latest_release(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    Ok(check_beefex_update(&state).await)
}

async fn check_beefex_update(state: &AppState) -> serde_json::Value {
    let current = env!("CARGO_PKG_VERSION");
    let Some(platform) = current_platform_asset() else {
        return serde_json::json!({
            "available": false,
            "updatesDisabled": true,
            "reason": "unsupported_platform",
        });
    };

    match resolve_latest_release(state, &platform).await {
        Ok(release) => {
            let available = is_newer_version(&release.version, current);
            serde_json::json!({
                "available": available,
                "version": release.version,
                "tag": release.tag,
                "htmlUrl": release.html_url,
                "body": release.notes,
                "publishedAt": release.published_at,
                "sha256": release.sha256,
                "assetName": release.asset_name,
                "source": release.source,
                "commit": release.commit,
            })
        }
        Err(_) => serde_json::json!({
            "available": false,
            "checkFailed": true,
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlatformAsset {
    key: &'static str,
    file: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedRelease {
    version: String,
    tag: String,
    html_url: String,
    notes: String,
    published_at: String,
    sha256: String,
    asset_name: String,
    source: &'static str,
    commit: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct BeefexUpdaterDocument {
    schema_version: String,
    product: String,
    identifier: String,
    version: String,
    #[serde(default)]
    tag: String,
    #[serde(default)]
    source_commit: String,
    #[serde(default)]
    notes: serde_json::Value,
    #[serde(default)]
    assets: HashMap<String, BeefexUpdaterAsset>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct BeefexUpdaterAsset {
    #[serde(default)]
    file: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    sha256: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct GithubRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    published_at: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct GithubReleaseAsset {
    #[serde(default)]
    name: String,
    #[serde(default)]
    browser_download_url: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct AlphaArtifact {
    schema_version: String,
    #[serde(default)]
    #[allow(dead_code)]
    tag: String,
    #[serde(default)]
    source_commit: String,
    #[serde(default)]
    artifact: String,
    #[serde(default)]
    sha256: String,
}

fn current_platform_asset() -> Option<PlatformAsset> {
    platform_asset(std::env::consts::OS, std::env::consts::ARCH)
}

fn platform_asset(os: &str, arch: &str) -> Option<PlatformAsset> {
    match (os, arch) {
        ("macos", "aarch64") => Some(PlatformAsset {
            key: "macos-aarch64",
            file: "beefex-desktop-mac-arm64.dmg",
        }),
        ("windows", "x86_64") => Some(PlatformAsset {
            key: "windows-x86_64",
            file: "beefex-desktop-win-x64.exe",
        }),
        _ => None,
    }
}

fn updater_base_url() -> String {
    std::env::var("BEEFEX_UPDATE_BASE")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| R2_LATEST_BASE.to_string())
}

fn github_releases_url() -> String {
    std::env::var("BEEFEX_UPDATE_RELEASES_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| GITHUB_RELEASES_URL.to_string())
}

async fn resolve_latest_release(
    state: &AppState,
    platform: &PlatformAsset,
) -> Result<ResolvedRelease, String> {
    let sums = fetch_text(state, &format!("{}/SHA256SUMS.txt", updater_base_url())).await?;
    let checksums = parse_sha256sums(&sums).ok_or_else(|| "invalid_sha256sums".to_string())?;
    let expected = checksums
        .get(platform.file)
        .cloned()
        .ok_or_else(|| "platform_asset_missing".to_string())?;

    if let Some(release) = try_r2_updater_document(state, platform, &expected).await {
        return Ok(release);
    }
    compose_from_github_artifact(state, platform, &expected).await
}

async fn try_r2_updater_document(
    state: &AppState,
    platform: &PlatformAsset,
    expected_sha256: &str,
) -> Option<ResolvedRelease> {
    let body = fetch_text(
        state,
        &format!("{}/beefex-updater.json", updater_base_url()),
    )
    .await
    .ok()?;
    let document = parse_beefex_updater_document(&body)?;
    let asset = document.assets.get(platform.key)?;
    let file = if asset.file.is_empty() {
        platform.file
    } else {
        asset.file.as_str()
    };
    if file != platform.file {
        return None;
    }
    if !same_sha256(&asset.sha256, expected_sha256) {
        return None;
    }
    if !asset.url.is_empty() && !asset.url.contains("/beefex/releases/latest/") {
        return None;
    }
    let version = document.version;
    let tag = if document.tag.is_empty() {
        format!("v{version}")
    } else {
        document.tag
    };
    Some(ResolvedRelease {
        version,
        tag,
        html_url: PUBLIC_DOWNLOAD_PAGE.to_string(),
        notes: notes_from_value(&document.notes),
        published_at: String::new(),
        sha256: expected_sha256.to_string(),
        asset_name: platform.file.to_string(),
        source: "r2-updater",
        commit: document.source_commit,
    })
}

async fn compose_from_github_artifact(
    state: &AppState,
    platform: &PlatformAsset,
    expected_sha256: &str,
) -> Result<ResolvedRelease, String> {
    let releases_json = fetch_text(state, &github_releases_url()).await?;
    let releases: Vec<GithubRelease> =
        serde_json::from_str(&releases_json).map_err(|_| "invalid_github_releases".to_string())?;
    let release = pick_latest_github_release(&releases)
        .ok_or_else(|| "github_release_missing".to_string())?;
    let version = normalize_release_version(&release.tag_name)
        .ok_or_else(|| "invalid_release_tag".to_string())?;
    let artifact_name = if platform.file.ends_with(".dmg") {
        "beefex-alpha-artifact.json"
    } else {
        "beefex-windows-alpha-artifact.json"
    };
    let artifact_url = release
        .assets
        .iter()
        .find(|asset| asset.name == artifact_name)
        .map(|asset| asset.browser_download_url.clone())
        .ok_or_else(|| "github_artifact_missing".to_string())?;
    let artifact_json = fetch_text(state, &artifact_url).await?;
    let artifact =
        parse_alpha_artifact(&artifact_json).ok_or_else(|| "invalid_alpha_artifact".to_string())?;
    if artifact.artifact != platform.file || !same_sha256(&artifact.sha256, expected_sha256) {
        return Err("r2_checksum_mismatch".to_string());
    }
    if let Some(sums_url) = release
        .assets
        .iter()
        .find(|asset| asset.name == "SHA256SUMS.txt")
        .map(|asset| asset.browser_download_url.as_str())
    {
        if let Ok(github_sums) = fetch_text(state, sums_url).await {
            if let Some(github_checksums) = parse_sha256sums(&github_sums) {
                if !github_checksums
                    .get(platform.file)
                    .is_some_and(|hash| same_sha256(hash, expected_sha256))
                {
                    return Err("r2_checksum_mismatch".to_string());
                }
            }
        }
    }
    Ok(ResolvedRelease {
        version,
        tag: release.tag_name.clone(),
        html_url: if release.html_url.is_empty() {
            PUBLIC_DOWNLOAD_PAGE.to_string()
        } else {
            release.html_url.clone()
        },
        notes: release.body.clone(),
        published_at: release.published_at.clone(),
        sha256: expected_sha256.to_string(),
        asset_name: platform.file.to_string(),
        source: "github-artifact+r2",
        commit: artifact.source_commit,
    })
}

fn parse_beefex_updater_document(body: &str) -> Option<BeefexUpdaterDocument> {
    let document: BeefexUpdaterDocument = serde_json::from_str(body).ok()?;
    if document.schema_version != UPDATER_SCHEMA
        || document.product != PRODUCT_NAME
        || document.identifier != PRODUCT_IDENTIFIER
    {
        return None;
    }
    normalize_release_version(&document.version)?;
    Some(document)
}

fn parse_alpha_artifact(body: &str) -> Option<AlphaArtifact> {
    let artifact: AlphaArtifact = serde_json::from_str(body).ok()?;
    if artifact.schema_version != ARTIFACT_SCHEMA
        || artifact.artifact.is_empty()
        || !is_sha256(&artifact.sha256)
    {
        return None;
    }
    Some(artifact)
}

fn pick_latest_github_release(releases: &[GithubRelease]) -> Option<&GithubRelease> {
    releases
        .iter()
        .filter(|release| !release.draft && normalize_release_version(&release.tag_name).is_some())
        .max_by(|left, right| {
            let left_version = normalize_release_version(&left.tag_name).unwrap();
            let right_version = normalize_release_version(&right.tag_name).unwrap();
            match (
                is_newer_version(&left_version, &right_version),
                is_newer_version(&right_version, &left_version),
            ) {
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => std::cmp::Ordering::Equal,
            }
        })
}

fn parse_sha256sums(text: &str) -> Option<HashMap<String, String>> {
    let mut checksums = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (hash, name) = line.split_once(char::is_whitespace)?;
        let hash = hash.trim().to_ascii_lowercase();
        let name = name.trim().trim_start_matches('*').trim();
        if !is_sha256(&hash) || name.is_empty() {
            return None;
        }
        checksums.insert(name.to_string(), hash);
    }
    if checksums.is_empty() {
        None
    } else {
        Some(checksums)
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn same_sha256(left: &str, right: &str) -> bool {
    is_sha256(left) && left.eq_ignore_ascii_case(right)
}

fn notes_from_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn user_agent() -> String {
    format!("Beefex/{}", env!("CARGO_PKG_VERSION"))
}

async fn fetch_text(state: &AppState, url: &str) -> Result<String, String> {
    let response = with_standard_request_timeout(
        state
            .http
            .get(url)
            .header("User-Agent", user_agent())
            .header("Accept", "application/json, text/plain;q=0.9, */*;q=0.8"),
    )
    .send()
    .await
    .map_err(|_| "update_check_network_failed".to_string())?;
    if !response.status().is_success() {
        return Err(format!("update_check_http_{}", response.status().as_u16()));
    }
    response
        .text()
        .await
        .map_err(|_| "update_check_body_invalid".to_string())
}

/// 比较 Beefex Alpha 版本。
///
/// `0.1.0` 无预发布后缀时视为早于 `0.1.0-alpha.N`。这只是比较器语义。
/// 已发布的 Alpha 4 把 updater 编译期关掉了，不能靠这个比较自动发现 Alpha 5。
fn is_newer_version(latest: &str, current: &str) -> bool {
    let Some(latest) = parse_version(latest) else {
        return false;
    };
    let Some(current) = parse_version(current) else {
        return true;
    };
    latest > current
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedVersion {
    major: u32,
    minor: u32,
    patch: u32,
    prerelease: Option<Vec<PrereleasePart>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrereleasePart {
    Number(u32),
    Text(String),
}

impl PartialOrd for ParsedVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ParsedVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(parts)) if is_untagged_alpha_predecessor(self, parts) => {
                    std::cmp::Ordering::Less
                }
                (Some(parts), None) if is_untagged_alpha_predecessor(other, parts) => {
                    std::cmp::Ordering::Greater
                }
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(left), Some(right)) => compare_prerelease(left, right),
            })
    }
}

fn is_untagged_alpha_predecessor(version: &ParsedVersion, parts: &[PrereleasePart]) -> bool {
    version.major == 0
        && version.minor == 1
        && version.patch == 0
        && matches!(parts.first(), Some(PrereleasePart::Text(label)) if label == "alpha")
}

fn compare_prerelease(left: &[PrereleasePart], right: &[PrereleasePart]) -> std::cmp::Ordering {
    for (left_part, right_part) in left.iter().zip(right.iter()) {
        let order = match (left_part, right_part) {
            (PrereleasePart::Number(a), PrereleasePart::Number(b)) => a.cmp(b),
            (PrereleasePart::Text(a), PrereleasePart::Text(b)) => a.cmp(b),
            (PrereleasePart::Number(_), PrereleasePart::Text(_)) => std::cmp::Ordering::Less,
            (PrereleasePart::Text(_), PrereleasePart::Number(_)) => std::cmp::Ordering::Greater,
        };
        if order != std::cmp::Ordering::Equal {
            return order;
        }
    }
    left.len().cmp(&right.len())
}

fn parse_version(raw: &str) -> Option<ParsedVersion> {
    let version = normalize_release_version(raw)?;
    let (core, prerelease) = version
        .split_once('-')
        .map(|(core, prerelease)| (core, Some(prerelease)))
        .unwrap_or((version.as_str(), None));
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let prerelease = prerelease.map(|value| {
        value
            .split('.')
            .map(|part| {
                if let Ok(number) = part.parse::<u32>() {
                    PrereleasePart::Number(number)
                } else {
                    PrereleasePart::Text(part.to_string())
                }
            })
            .collect()
    });
    Some(ParsedVersion {
        major,
        minor,
        patch,
        prerelease,
    })
}

fn normalize_release_version(version: &str) -> Option<String> {
    let trimmed = version.trim();
    let version = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let (core, prerelease) = version
        .split_once('-')
        .map_or((version, None), |(core, prerelease)| {
            (core, Some(prerelease))
        });
    let core_parts: Vec<&str> = core.split('.').collect();
    if core_parts.len() != 3
        || core_parts
            .iter()
            .any(|part| part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    if let Some(prerelease) = prerelease {
        if prerelease.split('.').any(|part| {
            part.is_empty() || !part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        }) {
            return None;
        }
    }
    Some(version.to_string())
}

fn latest_asset_url(file: &str) -> String {
    format!("{}/{file}", updater_base_url())
}

#[cfg(test)]
fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn normalize_path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn path_touches_user_data(path: &Path) -> bool {
    normalize_path_key(path).contains(USER_DATA_MARKER)
}

#[cfg(any(target_os = "macos", test))]
fn is_real_nonsymlink_app(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    !metadata.file_type().is_symlink()
        && metadata.is_dir()
        && path.extension().and_then(|ext| ext.to_str()) == Some("app")
}

#[cfg(any(target_os = "macos", test))]
fn discover_unique_real_app(mount_point: &Path) -> Result<PathBuf, String> {
    let mut apps = Vec::new();
    for entry in fs::read_dir(mount_point).map_err(|e| format!("读取挂载点失败: {e}"))? {
        let path = entry.map_err(|e| format!("读取挂载点失败: {e}"))?.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("app") {
            apps.push(path);
        }
    }
    if apps.len() != 1 {
        return Err(format!("DMG 必须恰好包含一个 .app，实际 {}", apps.len()));
    }
    let app = apps.remove(0);
    if app.file_name().and_then(|name| name.to_str()) != Some("Beefex.app") {
        return Err("DMG 内不是 Beefex.app".to_string());
    }
    if !is_real_nonsymlink_app(&app) {
        return Err("DMG 内的 Beefex.app 必须是真实目录，不能是符号链接".to_string());
    }
    Ok(app)
}

#[cfg(any(target_os = "macos", test))]
fn plist_string_value(plist: &str, key: &str) -> Option<String> {
    let marker = format!("<key>{key}</key>");
    let rest = plist.split_once(&marker)?.1;
    let start = rest.find("<string>")? + "<string>".len();
    let end = rest[start..].find("</string>")?;
    Some(rest[start..start + end].trim().to_string())
}

#[cfg(any(target_os = "macos", test))]
fn parse_bundle_identity(plist: &str) -> Result<(String, String), String> {
    let identifier = plist_string_value(plist, "CFBundleIdentifier")
        .ok_or_else(|| "staged bundle missing CFBundleIdentifier".to_string())?;
    let version = plist_string_value(plist, "CFBundleShortVersionString")
        .or_else(|| plist_string_value(plist, "CFBundleVersion"))
        .ok_or_else(|| "staged bundle missing version".to_string())?;
    Ok((identifier, version))
}

#[cfg(any(target_os = "macos", test))]
fn verify_staged_identity(
    identifier: &str,
    version: &str,
    expected_version: &str,
) -> Result<(), String> {
    if identifier != PRODUCT_IDENTIFIER {
        return Err(format!(
            "staged bundle id {identifier} is not {PRODUCT_IDENTIFIER}"
        ));
    }
    let actual = normalize_release_version(version)
        .ok_or_else(|| format!("staged version invalid: {version}"))?;
    let expected = normalize_release_version(expected_version)
        .ok_or_else(|| format!("expected version invalid: {expected_version}"))?;
    if actual != expected {
        return Err(format!("staged version {actual} != {expected}"));
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct MacosSwapPlan {
    staged_app: PathBuf,
    target_app: PathBuf,
    backup_app: PathBuf,
    failed_app: PathBuf,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct MacosRollbackMoves {
    target_to_failed: bool,
    backup_to_target: bool,
}

#[cfg(any(target_os = "macos", test))]
fn plan_macos_swap(applications: &Path, swap_id: &str) -> Result<MacosSwapPlan, String> {
    if swap_id.is_empty()
        || swap_id.contains('/')
        || swap_id.contains('\\')
        || swap_id.contains("..")
    {
        return Err("invalid macos swap id".to_string());
    }
    let plan = MacosSwapPlan {
        staged_app: applications.join(format!(".Beefex.staged-{swap_id}.app")),
        target_app: applications.join("Beefex.app"),
        backup_app: applications.join(format!(".Beefex.previous-{swap_id}.app")),
        failed_app: applications.join(format!(".Beefex.failed-{swap_id}.app")),
    };
    let parents = [
        plan.staged_app.parent(),
        plan.target_app.parent(),
        plan.backup_app.parent(),
        plan.failed_app.parent(),
    ];
    if parents.iter().any(|parent| *parent != Some(applications)) {
        return Err("macos swap paths must stay in the same directory".to_string());
    }
    for path in [
        &plan.staged_app,
        &plan.target_app,
        &plan.backup_app,
        &plan.failed_app,
    ] {
        if path_touches_user_data(path) {
            return Err("拒绝写入用户数据目录".to_string());
        }
    }
    Ok(plan)
}

#[cfg(any(target_os = "macos", test))]
fn plan_macos_rollback(
    target_exists: bool,
    backup_exists: bool,
) -> Result<MacosRollbackMoves, String> {
    if !backup_exists {
        return Err("没有可恢复的备份".to_string());
    }
    Ok(MacosRollbackMoves {
        target_to_failed: target_exists,
        backup_to_target: true,
    })
}

#[cfg(any(target_os = "macos", test))]
fn restore_macos_backup(plan: &MacosSwapPlan) -> Result<(), String> {
    let moves = plan_macos_rollback(plan.target_app.exists(), plan.backup_app.exists())?;
    if moves.target_to_failed {
        fs::rename(&plan.target_app, &plan.failed_app)
            .map_err(|e| format!("无法把失败的新版本移到同目录: {e}"))?;
    }
    fs::rename(&plan.backup_app, &plan.target_app)
        .map_err(|e| format!("回滚上一版 Beefex.app 失败: {e}"))
}

#[cfg(any(target_os = "windows", test))]
const WINDOWS_SILENT_NSIS_ARG: &str = "/S";

#[cfg(test)]
const WINDOWS_CURRENT_PROCESS_EXIT_WAIT_SECS: u32 = 120;

#[cfg(test)]
const WINDOWS_CURRENT_PROCESS_EXIT_TIMEOUT_MARKER: &str = "current_process_exit_timeout";

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowsSilentUpdatePlan {
    installer: PathBuf,
    current_exe: PathBuf,
    relaunch_exe: PathBuf,
    installer_args: Vec<String>,
}

#[cfg(any(target_os = "windows", test))]
fn windows_silent_nsis_args() -> Vec<String> {
    vec![WINDOWS_SILENT_NSIS_ARG.to_string()]
}

#[cfg(any(target_os = "windows", test))]
fn args_request_application_data_deletion(args: &[String]) -> bool {
    args.iter().any(|arg| {
        let trimmed = arg.trim();
        let lower = trimmed.to_ascii_lowercase();
        trimmed == "/P"
            || trimmed == "/p"
            || lower == "-p"
            || lower.contains("purge")
            || lower.contains("delete-application-data")
            || lower.contains("delete_application_data")
            || lower.contains("delete-appdata")
            || lower.contains("delete_appdata")
    })
}

#[cfg(any(target_os = "windows", test))]
fn ensure_windows_installer_args_safe(args: &[String]) -> Result<(), String> {
    if args.len() != 1 || args[0] != WINDOWS_SILENT_NSIS_ARG {
        return Err("Windows 静默安装参数必须恰好是 /S".to_string());
    }
    if args_request_application_data_deletion(args) {
        return Err("拒绝带数据删除参数的安装".to_string());
    }
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
fn plan_windows_silent_update(
    installer: &Path,
    current_exe: &Path,
) -> Result<WindowsSilentUpdatePlan, String> {
    if !installer.exists() {
        return Err(format!("安装包不存在: {}", installer.display()));
    }
    if path_touches_user_data(installer) {
        return Err("拒绝从用户数据目录安装".to_string());
    }
    if current_exe.as_os_str().is_empty() {
        return Err("无法解析当前 Beefex 路径".to_string());
    }
    if !current_exe.exists() {
        return Err(format!("当前 Beefex 不存在: {}", current_exe.display()));
    }
    if path_touches_user_data(current_exe) {
        return Err("拒绝从用户数据目录启动当前 Beefex".to_string());
    }
    let installer_args = windows_silent_nsis_args();
    ensure_windows_installer_args_safe(&installer_args)?;
    Ok(WindowsSilentUpdatePlan {
        installer: installer.to_path_buf(),
        current_exe: current_exe.to_path_buf(),
        relaunch_exe: current_exe.to_path_buf(),
        installer_args,
    })
}

#[cfg(any(target_os = "windows", test))]
fn windows_silent_update_waiter_script() -> &'static str {
    r#"
$ErrorActionPreference = 'Stop'
$appPid = [int]$env:BEEFEX_UPDATE_PID
$installer = $env:BEEFEX_UPDATE_INSTALLER
$relaunch = $env:BEEFEX_UPDATE_RELAUNCH
$failure = $env:BEEFEX_UPDATE_FAILURE
$nsisArgs = $env:BEEFEX_UPDATE_NSIS_ARGS
if (-not $installer -or -not $relaunch -or -not $failure -or -not $nsisArgs) {
  exit 1
}
if ($nsisArgs -cne '/S') {
  Set-Content -LiteralPath $failure -Value 'invalid_installer_args' -Encoding ascii
  exit 1
}
$deadline = (Get-Date).AddSeconds(120)
while ($true) {
  if (-not (Get-Process -Id $appPid -ErrorAction SilentlyContinue)) {
    break
  }
  if ((Get-Date) -ge $deadline) {
    Set-Content -LiteralPath $failure -Value 'current_process_exit_timeout' -Encoding ascii
    exit 1
  }
  Start-Sleep -Seconds 1
}
try {
  $p = Start-Process -FilePath $installer -ArgumentList $nsisArgs -Wait -PassThru
} catch {
  Set-Content -LiteralPath $failure -Value 'installer_spawn_failed' -Encoding ascii
  exit 1
}
if ($null -eq $p) {
  Set-Content -LiteralPath $failure -Value 'installer_spawn_failed' -Encoding ascii
  exit 1
}
if ($p.ExitCode -ne 0) {
  Set-Content -LiteralPath $failure -Value ('installer_exit=' + $p.ExitCode) -Encoding ascii
  exit $p.ExitCode
}
try {
  $null = Start-Process -FilePath $relaunch
} catch {
  Set-Content -LiteralPath $failure -Value 'relaunch_failed' -Encoding ascii
  exit 1
}
if (Test-Path -LiteralPath $failure) {
  Remove-Item -LiteralPath $failure -Force -ErrorAction SilentlyContinue
}
exit 0
"#
}

#[cfg(target_os = "windows")]
fn spawn_windows_silent_update_waiter(
    plan: &WindowsSilentUpdatePlan,
    current_pid: u32,
) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    use crate::proc::CREATE_NO_WINDOW;

    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let failure_record = std::env::temp_dir().join("beefex-update-failure.txt");
    if path_touches_user_data(&failure_record) {
        return Err("拒绝把更新失败记录写进用户数据目录".to_string());
    }
    if plan.relaunch_exe != plan.current_exe {
        return Err("Windows 重启动路径必须是当前 Beefex".to_string());
    }
    let mut cmd = std::process::Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-WindowStyle",
        "Hidden",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        windows_silent_update_waiter_script(),
    ])
    .env("BEEFEX_UPDATE_PID", current_pid.to_string())
    .env("BEEFEX_UPDATE_INSTALLER", &plan.installer)
    .env("BEEFEX_UPDATE_RELAUNCH", &plan.relaunch_exe)
    .env("BEEFEX_UPDATE_FAILURE", &failure_record)
    .env("BEEFEX_UPDATE_NSIS_ARGS", plan.installer_args.join(" "))
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());
    cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    cmd.spawn()
        .map_err(|e| format!("启动静默更新等待进程失败: {e}"))?;
    Ok(())
}

/// 下载新版本安装包到 OS temp dir，边下边校验 SHA-256，并 emit "update-download-progress"。
#[tauri::command]
pub(crate) async fn download_update_asset(
    app: AppHandle,
    state: State<'_, AppState>,
    version: String,
    sha256: Option<String>,
) -> Result<String, String> {
    let version = normalize_release_version(&version)
        .ok_or_else(|| format!("无效的 release 版本号: {version}"))?;
    let platform = current_platform_asset().ok_or_else(|| {
        format!(
            "没有匹配当前平台({}/{})的安装包",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let sums = fetch_text(&state, &format!("{}/SHA256SUMS.txt", updater_base_url())).await?;
    let checksums =
        parse_sha256sums(&sums).ok_or_else(|| "无法解析 R2 SHA256SUMS.txt".to_string())?;
    let expected = checksums
        .get(platform.file)
        .cloned()
        .ok_or_else(|| format!("SHA256SUMS 缺少 {}", platform.file))?;
    if let Some(provided) = sha256.as_deref() {
        if !same_sha256(provided, &expected) {
            return Err("更新元数据与 R2 SHA256SUMS 不一致".to_string());
        }
    }
    let dest = std::env::temp_dir().join(format!("beefex-update-{version}-{}", platform.file));
    if path_touches_user_data(&dest) {
        return Err("拒绝写入用户数据目录".to_string());
    }
    let asset_url = latest_asset_url(platform.file);
    let mut resp = state
        .http
        .get(&asset_url)
        .header("User-Agent", user_agent())
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载返回 {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file = fs::File::create(&dest).map_err(|e| format!("创建文件失败: {e}"))?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut last_emitted_pct: i32 = -1;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("读取下载流失败: {e}"))?
    {
        file.write_all(&chunk)
            .map_err(|e| format!("写入失败: {e}"))?;
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;
        let pct = if total > 0 {
            (downloaded * 100 / total) as i32
        } else {
            0
        };
        if pct != last_emitted_pct {
            last_emitted_pct = pct;
            let _ = app.emit(
                "update-download-progress",
                serde_json::json!({
                  "percent": pct,
                  "downloadedBytes": downloaded,
                  "totalBytes": total,
                }),
            );
        }
    }
    let actual = hex_encode(&hasher.finalize());
    if !same_sha256(&actual, &expected) {
        let _ = fs::remove_file(&dest);
        return Err("下载文件 SHA-256 与 R2 SHA256SUMS 不一致".to_string());
    }
    let _ = app.emit(
        "update-download-progress",
        serde_json::json!({
          "percent": 100,
          "downloadedBytes": downloaded,
          "totalBytes": total.max(downloaded),
        }),
    );
    Ok(dest.to_string_lossy().to_string())
}

#[cfg(target_os = "macos")]
fn detach_dmg(mount_str: &str, mount_point: &Path) {
    let _ = std::process::Command::new("hdiutil")
        .args(["detach", "-force", mount_str])
        .status();
    let _ = fs::remove_dir(mount_point);
}

/// 启动安装包并退出当前应用。
/// macOS：只读挂载 → 暂存校验 → rename 交换；失败回滚备份。不删 Application Support / AppData。
/// Windows：校验路径后拉起内置 PowerShell 等待进程，退出当前应用，以恰好 `/S` 静默安装，成功后再启动已安装 Beefex。不传数据删除参数。
#[tauri::command]
pub(crate) fn install_update_and_quit(
    app: AppHandle,
    path: String,
    version: Option<String>,
) -> Result<(), String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("安装包不存在: {path}"));
    }
    if path_touches_user_data(p) {
        return Err("拒绝从用户数据目录安装".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let expected_version = version
            .as_deref()
            .and_then(normalize_release_version)
            .ok_or_else(|| "macOS 安装需要有效的目标版本号".to_string())?;
        let mount_id = Uuid::new_v4().simple().to_string();
        let mount_point = std::env::temp_dir().join(format!("beefex-mount-{mount_id}"));
        fs::create_dir_all(&mount_point).map_err(|e| format!("创建挂载目录失败: {e}"))?;
        let mount_str = mount_point.to_string_lossy().to_string();
        let attach = Command::new("hdiutil")
            .args([
                "attach",
                "-nobrowse",
                "-readonly",
                "-mountpoint",
                &mount_str,
                &path,
            ])
            .output()
            .map_err(|e| format!("hdiutil attach 失败: {e}"))?;
        if !attach.status.success() {
            let _ = fs::remove_dir(&mount_point);
            return Err(format!(
                "挂载 DMG 失败: {}",
                String::from_utf8_lossy(&attach.stderr)
            ));
        }
        let app_in_dmg = match discover_unique_real_app(&mount_point) {
            Ok(app) => app,
            Err(reason) => {
                detach_dmg(&mount_str, &mount_point);
                return Err(reason);
            }
        };
        let swap_id = Uuid::new_v4().simple().to_string();
        let plan = match plan_macos_swap(Path::new("/Applications"), &swap_id) {
            Ok(plan) => plan,
            Err(reason) => {
                detach_dmg(&mount_str, &mount_point);
                return Err(reason);
            }
        };
        if plan.staged_app.exists() {
            detach_dmg(&mount_str, &mount_point);
            return Err("暂存安装路径已存在".to_string());
        }
        let ditto = Command::new("ditto")
            .args([&app_in_dmg, &plan.staged_app])
            .status();
        let ditto_ok = matches!(ditto, Ok(status) if status.success());
        if !ditto_ok {
            let _ = fs::remove_dir_all(&plan.staged_app);
            detach_dmg(&mount_str, &mount_point);
            return Err("复制暂存 Beefex.app 失败".to_string());
        }
        let plist_path = plan.staged_app.join("Contents/Info.plist");
        let identity = fs::read_to_string(&plist_path)
            .map_err(|e| format!("读取暂存 Info.plist 失败: {e}"))
            .and_then(|plist| parse_bundle_identity(&plist))
            .and_then(|(identifier, staged_version)| {
                verify_staged_identity(&identifier, &staged_version, &expected_version)
            });
        if let Err(reason) = identity {
            let _ = fs::remove_dir_all(&plan.staged_app);
            detach_dmg(&mount_str, &mount_point);
            return Err(reason);
        }
        let target_existed = plan.target_app.exists();
        if target_existed {
            if plan.backup_app.exists() {
                let _ = fs::remove_dir_all(&plan.staged_app);
                detach_dmg(&mount_str, &mount_point);
                return Err("可回滚备份路径已存在".to_string());
            }
            if let Err(error) = fs::rename(&plan.target_app, &plan.backup_app) {
                let _ = fs::remove_dir_all(&plan.staged_app);
                detach_dmg(&mount_str, &mount_point);
                return Err(format!("无法把当前应用移到备份位置: {error}"));
            }
        }
        if let Err(error) = fs::rename(&plan.staged_app, &plan.target_app) {
            if target_existed {
                let _ = restore_macos_backup(&plan);
            }
            let _ = fs::remove_dir_all(&plan.staged_app);
            detach_dmg(&mount_str, &mount_point);
            return Err(format!("无法把暂存应用换到 /Applications: {error}"));
        }
        detach_dmg(&mount_str, &mount_point);
        if let Err(error) = Command::new("open")
            .args(["-n", &plan.target_app.to_string_lossy()])
            .spawn()
        {
            if target_existed {
                let _ = restore_macos_backup(&plan);
            }
            return Err(format!("open 新版本失败: {error}"));
        }
        app.exit(0);
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let _ = version;
        let current_exe =
            std::env::current_exe().map_err(|e| format!("无法解析当前 Beefex 路径: {e}"))?;
        let plan = plan_windows_silent_update(p, &current_exe)?;
        spawn_windows_silent_update_waiter(&plan, std::process::id())?;
        app.exit(0);
        return Ok(());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (app, version);
        Err("当前平台不支持自动安装".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALPHA4_MAC: &str = "9887c1dddc735d39bd064473316ab4cb7cd6fe7ad4ccb3170025ba14c488b1c9";
    const ALPHA4_WIN: &str = "38ba09c229f7beea1cac865ed1c22e54e48b45e6a3d4c2f43ee460d4ad2c1cee";

    fn sample_updater_json() -> String {
        format!(
            r#"{{
  "schema_version": "beefex.updater.v1",
  "product": "Beefex",
  "identifier": "com.beefapi.beefex",
  "version": "0.1.0-alpha.6",
  "tag": "v0.1.0-alpha.6",
  "channel": "alpha",
  "notes": ["next Alpha line"],
  "assets": {{
    "macos-aarch64": {{
      "file": "beefex-desktop-mac-arm64.dmg",
      "url": "https://pub-e540a6ea6d6e4af19d7f5fc4d1f07c47.r2.dev/beefex/releases/latest/beefex-desktop-mac-arm64.dmg",
      "sha256": "{ALPHA4_MAC}"
    }},
    "windows-x86_64": {{
      "file": "beefex-desktop-win-x64.exe",
      "url": "https://pub-e540a6ea6d6e4af19d7f5fc4d1f07c47.r2.dev/beefex/releases/latest/beefex-desktop-win-x64.exe",
      "sha256": "{ALPHA4_WIN}"
    }}
  }}
}}"#
        )
    }

    #[test]
    fn prerelease_versions_compare_in_alpha_order() {
        assert!(is_newer_version("0.1.0-alpha.4", "0.1.0-alpha.3"));
        assert!(is_newer_version("0.1.0-alpha.5", "0.1.0-alpha.4"));
        assert!(is_newer_version("0.1.0-alpha.6", "0.1.0-alpha.5"));
        assert!(!is_newer_version("0.1.0-alpha.6", "0.1.0-alpha.6"));
        assert!(!is_newer_version("0.1.0-alpha.5", "0.1.1"));
    }

    #[test]
    fn version_comparator_orders_untagged_010_before_alpha_prereleases() {
        // Comparator-only. Published Alpha 4 cannot auto-discover later builds.
        assert!(is_newer_version("0.1.0-alpha.5", "0.1.0"));
        assert!(is_newer_version("0.1.0-alpha.6", "0.1.0-alpha.5"));
        assert!(!is_newer_version("0.1.0", "0.1.0-alpha.5"));
    }

    #[test]
    fn parse_sha256sums_accepts_current_alpha4_manifest() {
        let text = format!("{ALPHA4_MAC}  beefex-desktop-mac-arm64.dmg\n{ALPHA4_WIN}  beefex-desktop-win-x64.exe\n");
        let checksums = parse_sha256sums(&text).unwrap();
        assert_eq!(
            checksums.get("beefex-desktop-mac-arm64.dmg").unwrap(),
            ALPHA4_MAC
        );
        assert_eq!(
            checksums.get("beefex-desktop-win-x64.exe").unwrap(),
            ALPHA4_WIN
        );
    }

    #[test]
    fn beefex_updater_document_is_accepted() {
        let document = parse_beefex_updater_document(&sample_updater_json()).unwrap();
        assert_eq!(document.version, "0.1.0-alpha.6");
        assert_eq!(
            document.assets.get("macos-aarch64").unwrap().file,
            "beefex-desktop-mac-arm64.dmg"
        );
    }

    #[test]
    fn stale_electron_latest_json_is_rejected() {
        let stale = r#"{
          "product": "Beefex Desktop",
          "version": "0.1.2",
          "channel": "beta",
          "commit": "86d6cc761-dirty-local",
          "assets": {
            "macos": { "file": "beefex-desktop-mac-arm64.dmg" }
          }
        }"#;
        assert!(parse_beefex_updater_document(stale).is_none());
    }

    #[test]
    fn latest_yml_identity_is_not_an_updater_document() {
        assert!(parse_beefex_updater_document(
            "version: 0.1.2\npath: beefex-desktop-mac-arm64.zip\n"
        )
        .is_none());
    }

    #[test]
    fn platform_assets_match_download_page_names() {
        assert_eq!(
            platform_asset("macos", "aarch64").unwrap().file,
            "beefex-desktop-mac-arm64.dmg"
        );
        assert_eq!(
            platform_asset("windows", "x86_64").unwrap().file,
            "beefex-desktop-win-x64.exe"
        );
        assert!(platform_asset("macos", "x86_64").is_none());
        assert!(platform_asset("linux", "x86_64").is_none());
    }

    #[test]
    fn alpha_artifact_receipt_is_source_traceable() {
        let artifact = parse_alpha_artifact(
            r#"{
              "schema_version": "beefex.alpha-artifact.v1",
              "tag": "v0.1.0-alpha.4",
              "source_commit": "1f826cc44549ab02227342c74785c1c290301b13",
              "artifact": "beefex-desktop-mac-arm64.dmg",
              "sha256": "9887c1dddc735d39bd064473316ab4cb7cd6fe7ad4ccb3170025ba14c488b1c9"
            }"#,
        )
        .unwrap();
        assert_eq!(
            artifact.source_commit,
            "1f826cc44549ab02227342c74785c1c290301b13"
        );
        assert!(same_sha256(&artifact.sha256, ALPHA4_MAC));
    }

    fn github_release() -> GithubRelease {
        GithubRelease {
            tag_name: String::new(),
            html_url: String::new(),
            body: String::new(),
            published_at: String::new(),
            draft: false,
            assets: Vec::new(),
        }
    }

    #[test]
    fn github_release_picker_skips_drafts_and_picks_newest_alpha() {
        let releases = vec![
            GithubRelease {
                tag_name: "v0.1.0-alpha.3".into(),
                ..github_release()
            },
            GithubRelease {
                tag_name: "v0.1.0-alpha.4".into(),
                ..github_release()
            },
            GithubRelease {
                tag_name: "v0.1.0-alpha.5".into(),
                draft: true,
                ..github_release()
            },
        ];
        assert_eq!(
            pick_latest_github_release(&releases).unwrap().tag_name,
            "v0.1.0-alpha.4"
        );
    }

    #[test]
    fn install_targets_never_include_user_data_roots() {
        assert!(path_touches_user_data(Path::new(
            "/Users/test/Library/Application Support/com.beefapi.beefex/credentials/beefapi-managed"
        )));
        assert!(path_touches_user_data(Path::new(
            r"C:\Users\test\AppData\Roaming\com.beefapi.beefex\conversations"
        )));
        assert!(path_touches_user_data(Path::new(
            "/Users/test/Library/Application Support/com.beefapi.beefex/beefex-desktop-mac-arm64.dmg"
        )));
        assert!(path_touches_user_data(Path::new(
            r"C:\Users\test\AppData\Roaming\com.beefapi.beefex\beefex-desktop-win-x64.exe"
        )));
        assert!(!path_touches_user_data(Path::new(
            "/var/folders/xx/beefex-update-0.1.0-alpha.5-beefex-desktop-mac-arm64.dmg"
        )));
        assert!(!path_touches_user_data(Path::new(
            "/Applications/Beefex.app"
        )));
    }

    #[test]
    fn macos_swap_plan_keeps_all_renames_in_applications() {
        let applications = Path::new("/Applications");
        let plan = plan_macos_swap(applications, "swap1").unwrap();
        assert_eq!(plan.target_app, PathBuf::from("/Applications/Beefex.app"));
        assert_eq!(
            plan.staged_app,
            PathBuf::from("/Applications/.Beefex.staged-swap1.app")
        );
        assert_eq!(
            plan.backup_app,
            PathBuf::from("/Applications/.Beefex.previous-swap1.app")
        );
        assert_eq!(
            plan.failed_app,
            PathBuf::from("/Applications/.Beefex.failed-swap1.app")
        );
        for path in [
            &plan.staged_app,
            &plan.target_app,
            &plan.backup_app,
            &plan.failed_app,
        ] {
            assert_eq!(path.parent(), Some(applications));
        }
        assert!(plan
            .staged_app
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with('.'));
        assert!(plan
            .backup_app
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with('.'));
        assert!(plan
            .failed_app
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with('.'));
        assert!(plan_macos_swap(applications, "../x").is_err());
        assert!(plan_macos_swap(
            Path::new("/Users/test/Library/Application Support/com.beefapi.beefex"),
            "swap1",
        )
        .is_err());
    }

    #[test]
    fn macos_rollback_moves_failed_target_in_the_same_directory() {
        let moves = plan_macos_rollback(true, true).unwrap();
        assert_eq!(
            moves,
            MacosRollbackMoves {
                target_to_failed: true,
                backup_to_target: true,
            }
        );
        assert_eq!(
            plan_macos_rollback(false, true).unwrap(),
            MacosRollbackMoves {
                target_to_failed: false,
                backup_to_target: true,
            }
        );
        assert!(plan_macos_rollback(true, false).is_err());
    }

    #[test]
    fn staged_identity_requires_beefex_id_and_expected_version() {
        assert!(
            verify_staged_identity("com.beefapi.beefex", "0.1.0-alpha.6", "v0.1.0-alpha.6").is_ok()
        );
        assert!(
            verify_staged_identity("com.apple.finder", "0.1.0-alpha.6", "0.1.0-alpha.6").is_err()
        );
        assert!(
            verify_staged_identity("com.beefapi.beefex", "0.1.0-alpha.5", "0.1.0-alpha.6").is_err()
        );
    }

    #[test]
    fn bundle_plist_parser_reads_identifier_and_short_version() {
        let plist = r#"
            <dict>
              <key>CFBundleIdentifier</key>
              <string>com.beefapi.beefex</string>
              <key>CFBundleShortVersionString</key>
              <string>0.1.0-alpha.6</string>
            </dict>
        "#;
        assert_eq!(
            parse_bundle_identity(plist).unwrap(),
            (
                "com.beefapi.beefex".to_string(),
                "0.1.0-alpha.6".to_string()
            )
        );
    }

    #[test]
    fn discover_unique_real_app_rejects_symlinks_and_duplicates() {
        let root =
            std::env::temp_dir().join(format!("beefex-app-disc-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(root.join("Beefex.app")).unwrap();
        assert_eq!(
            discover_unique_real_app(&root)
                .unwrap()
                .file_name()
                .unwrap(),
            "Beefex.app"
        );
        fs::create_dir_all(root.join("Other.app")).unwrap();
        assert!(discover_unique_real_app(&root).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn discover_unique_real_app_rejects_symlink_app() {
        let linked =
            std::env::temp_dir().join(format!("beefex-app-link-{}", Uuid::new_v4().simple()));
        let target =
            std::env::temp_dir().join(format!("beefex-app-real-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(target.join("Beefex.app")).unwrap();
        fs::create_dir_all(&linked).unwrap();
        std::os::unix::fs::symlink(target.join("Beefex.app"), linked.join("Beefex.app")).unwrap();
        assert!(discover_unique_real_app(&linked).is_err());
        fs::remove_dir_all(&linked).unwrap();
        fs::remove_dir_all(&target).unwrap();
    }

    #[test]
    fn sha256_helper_matches_known_empty_digest() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn normalize_release_version_accepts_alpha_tags() {
        assert_eq!(
            normalize_release_version("v0.1.0-alpha.6").as_deref(),
            Some("0.1.0-alpha.6")
        );
        assert_eq!(
            normalize_release_version("v0.1.0-alpha.5").as_deref(),
            Some("0.1.0-alpha.5")
        );
        assert_eq!(normalize_release_version("0.1.0-alpha.6/asset"), None);
    }

    fn temp_windows_update_files() -> (PathBuf, PathBuf, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("beefex-win-update-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&root).unwrap();
        let installer = root.join("beefex-desktop-win-x64.exe");
        let exe = root.join("Beefex.exe");
        fs::write(&installer, b"installer").unwrap();
        fs::write(&exe, b"beefex").unwrap();
        (root, installer, exe)
    }

    #[test]
    fn windows_silent_plan_uses_exact_nsis_s_flag_and_preserves_data() {
        let (root, installer, exe) = temp_windows_update_files();
        let plan = plan_windows_silent_update(&installer, &exe).unwrap();
        assert_eq!(plan.installer_args, vec!["/S".to_string()]);
        assert_ne!(plan.installer_args, vec!["/s".to_string()]);
        assert!(ensure_windows_installer_args_safe(&plan.installer_args).is_ok());
        assert!(!args_request_application_data_deletion(
            &plan.installer_args
        ));
        assert_eq!(plan.relaunch_exe, exe);
        assert_eq!(plan.current_exe, exe);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn windows_installer_args_reject_data_deletion_flags() {
        assert!(args_request_application_data_deletion(&[
            "/S".into(),
            "/P".into()
        ]));
        assert!(args_request_application_data_deletion(&[
            "/S".into(),
            "/purge".into()
        ]));
        assert!(args_request_application_data_deletion(&[
            "--delete-application-data".into()
        ]));
        assert!(!args_request_application_data_deletion(&["/S".into()]));
        assert!(ensure_windows_installer_args_safe(&["/S".into(), "/P".into()]).is_err());
        assert!(ensure_windows_installer_args_safe(&["/s".into()]).is_err());
    }

    #[test]
    fn windows_silent_plan_rejects_user_data_and_missing_paths() {
        let (root, installer, exe) = temp_windows_update_files();
        let missing_installer = root.join("missing-setup.exe");
        assert!(plan_windows_silent_update(&missing_installer, &exe).is_err());
        let missing_exe = root.join("missing-beefex.exe");
        assert!(plan_windows_silent_update(&installer, &missing_exe).is_err());

        let data_root = root.join("com.beefapi.beefex");
        fs::create_dir_all(&data_root).unwrap();
        let data_installer = data_root.join("setup.exe");
        fs::write(&data_installer, b"installer").unwrap();
        assert!(plan_windows_silent_update(&data_installer, &exe).is_err());

        let data_exe = data_root.join("Beefex.exe");
        fs::write(&data_exe, b"beefex").unwrap();
        assert!(plan_windows_silent_update(&installer, &data_exe).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn windows_waiter_script_is_bounded_silent_and_success_only() {
        let script = windows_silent_update_waiter_script();
        let timeout_at = script
            .find(WINDOWS_CURRENT_PROCESS_EXIT_TIMEOUT_MARKER)
            .expect("timeout marker");
        let installer_at = script
            .find("Start-Process -FilePath $installer")
            .expect("installer launch");
        let nonzero_at = script
            .find("$p.ExitCode -ne 0")
            .expect("non-zero installer exit");
        let relaunch_at = script
            .find("Start-Process -FilePath $relaunch")
            .expect("relaunch");

        assert!(script.contains("-cne '/S'"));
        assert!(script.contains("ArgumentList $nsisArgs"));
        assert!(script.contains(&format!(
            "AddSeconds({WINDOWS_CURRENT_PROCESS_EXIT_WAIT_SECS})"
        )));
        assert!(script.contains("Start-Sleep -Seconds 1"));
        assert!(!script.contains("Wait-Process"));
        assert!(timeout_at < installer_at);
        assert!(installer_at < nonzero_at);
        assert!(nonzero_at < relaunch_at);
        assert!(script.contains("exit 1"));
        assert!(!script.contains("/P"));
        assert!(!script.to_ascii_lowercase().contains("purge"));
        assert!(!script
            .to_ascii_lowercase()
            .contains("delete-application-data"));
        assert!(!script.to_ascii_lowercase().contains("delete_appdata"));
    }
}
