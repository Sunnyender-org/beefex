use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use regex::Regex;
use serde::{Deserialize, Serialize};
use tauri::{Manager, Runtime, State};

const MAX_MESSAGE_BYTES: usize = 512;
const DEFAULT_MAX_FILES: usize = 4;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 18 * 1024 * 1024;
const STORE_MAX_FILES: usize = 7;
const STORE_MAX_TOTAL_BYTES: u64 = 20 * 1024 * 1024;
const LOG_TARGET: &str = "beefex_diagnostics";

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticKind {
    Startup,
    AccountTransition,
    PiChildLifecycle,
    RunTerminal,
    TaskRecovery,
    RendererError,
    RustPanic,
    ExportLifecycle,
}

#[derive(Debug, Clone)]
pub struct DiagnosticEventInput<'a> {
    pub level: DiagnosticLevel,
    pub kind: DiagnosticKind,
    pub transition: Option<&'a str>,
    pub error_class: Option<&'a str>,
    pub message_code: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsPreview {
    pub categories: Vec<String>,
    pub excluded_categories: Vec<String>,
    pub file_count: usize,
    pub approximate_bytes: u64,
    pub app_version: String,
    pub first_timestamp: Option<String>,
    pub last_timestamp: Option<String>,
    pub skipped_records: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsExportReceipt {
    pub path: PathBuf,
    pub archive_bytes: u64,
    pub inventory: Vec<String>,
    pub manifest_schema_version: u8,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RendererDiagnosticInput {
    pub transition: String,
    pub error_class: String,
    pub message_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredDiagnosticEvent {
    schema_version: u8,
    timestamp: String,
    level: DiagnosticLevel,
    kind: DiagnosticKind,
    app_version: String,
    os_version: String,
    correlation_id: String,
    transition: Option<String>,
    error_class: Option<String>,
    message_code: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportManifest<'a> {
    schema_version: u8,
    generated_at: String,
    app_version: &'a str,
    os_version: &'a str,
    categories: &'a [String],
    excluded_categories: &'a [String],
    inventory: &'a [String],
    first_timestamp: &'a Option<String>,
    last_timestamp: &'a Option<String>,
    skipped_records: usize,
}

pub struct DiagnosticsService {
    root: PathBuf,
    app_version: String,
    os_version: String,
    correlation_id: String,
    max_files: usize,
    max_total_bytes: u64,
    writer: Mutex<()>,
}

impl DiagnosticsService {
    pub fn new(
        root: PathBuf,
        app_version: impl Into<String>,
        os_version: impl Into<String>,
    ) -> Self {
        Self {
            root,
            app_version: app_version.into(),
            os_version: os_version.into(),
            correlation_id: uuid::Uuid::new_v4().to_string(),
            max_files: DEFAULT_MAX_FILES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            writer: Mutex::new(()),
        }
    }

    #[cfg(test)]
    fn with_limits(mut self, max_files: usize, max_total_bytes: u64) -> Self {
        self.max_files = max_files;
        self.max_total_bytes = max_total_bytes;
        self
    }

    pub fn record(
        &self,
        input: DiagnosticEventInput<'_>,
        private_roots: &[PathBuf],
    ) -> Result<(), String> {
        let _guard = self
            .writer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        std::fs::create_dir_all(&self.root)
            .map_err(|_| "diagnostics_directory_create_failed".to_string())?;
        let event = StoredDiagnosticEvent {
            schema_version: 1,
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: input.level,
            kind: input.kind,
            app_version: self.app_version.clone(),
            os_version: self.os_version.clone(),
            correlation_id: self.correlation_id.clone(),
            transition: input
                .transition
                .map(|value| sanitize_message(value, private_roots)),
            error_class: input
                .error_class
                .map(|value| sanitize_message(value, private_roots)),
            message_code: input
                .message_code
                .map(|value| sanitize_message(value, private_roots)),
        };
        let mut serialized = serde_json::to_vec(&event)
            .map_err(|_| "diagnostics_event_serialize_failed".to_string())?;
        serialized.push(b'\n');
        self.rotate_if_needed(serialized.len() as u64)?;
        let path = self.root.join("events.ndjson");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|_| "diagnostics_event_write_failed".to_string())?;
        file.write_all(&serialized)
            .map_err(|_| "diagnostics_event_write_failed".to_string())?;
        file.flush()
            .map_err(|_| "diagnostics_event_write_failed".to_string())?;
        self.enforce_total_size()?;
        let message = format!(
            "kind={} transition={} error_class={} message_code={}",
            kind_name(input.kind),
            event.transition.as_deref().unwrap_or("none"),
            event.error_class.as_deref().unwrap_or("none"),
            event.message_code.as_deref().unwrap_or("none")
        );
        match input.level {
            DiagnosticLevel::Info => log::info!(target: LOG_TARGET, "{message}"),
            DiagnosticLevel::Warn => log::warn!(target: LOG_TARGET, "{message}"),
            DiagnosticLevel::Error => log::error!(target: LOG_TARGET, "{message}"),
        }
        Ok(())
    }

    pub fn preview(&self) -> Result<DiagnosticsPreview, String> {
        let _guard = self
            .writer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let files = self.support_files()?;
        let mut categories = BTreeSet::new();
        let mut approximate_bytes = 0_u64;
        let mut first_timestamp: Option<String> = None;
        let mut last_timestamp: Option<String> = None;
        let mut skipped_records = 0;
        for path in &files {
            approximate_bytes = approximate_bytes.saturating_add(
                std::fs::metadata(path)
                    .map_err(|_| "diagnostics_preview_failed".to_string())?
                    .len(),
            );
            let file = File::open(path).map_err(|_| "diagnostics_preview_failed".to_string())?;
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if is_event_file(path) {
                    match serde_json::from_str::<StoredDiagnosticEvent>(&line) {
                        Ok(event) => {
                            categories.insert(kind_name(event.kind).to_string());
                            let timestamp = event.timestamp;
                            first_timestamp = Some(first_timestamp.map_or_else(
                                || timestamp.clone(),
                                |current| current.min(timestamp.clone()),
                            ));
                            last_timestamp = Some(last_timestamp.map_or_else(
                                || timestamp.clone(),
                                |current| current.max(timestamp.clone()),
                            ));
                        }
                        Err(_) => skipped_records += 1,
                    }
                }
            }
        }
        Ok(DiagnosticsPreview {
            categories: categories.into_iter().collect(),
            excluded_categories: excluded_categories(),
            file_count: files.len(),
            approximate_bytes,
            app_version: self.app_version.clone(),
            first_timestamp,
            last_timestamp,
            skipped_records,
        })
    }

    pub fn export(
        &self,
        path: &Path,
        private_roots: &[PathBuf],
    ) -> Result<DiagnosticsExportReceipt, String> {
        let _guard = self
            .writer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let files = self.support_files()?;
        let preview = self.preview_unlocked(&files)?;
        let inventory: Vec<String> = std::iter::once("manifest.json".to_string())
            .chain(
                files
                    .iter()
                    .enumerate()
                    .map(|(index, path)| archive_name(path, index)),
            )
            .collect();
        let temporary = path.with_extension(format!("zip.tmp-{}", uuid::Uuid::new_v4()));
        let result = self.write_archive(&temporary, &files, &preview, &inventory, private_roots);
        if let Err(error) = result {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
        std::fs::rename(&temporary, path).map_err(|_| {
            let _ = std::fs::remove_file(&temporary);
            "diagnostics_export_replace_failed".to_string()
        })?;
        let archive_bytes = std::fs::metadata(path)
            .map_err(|_| "diagnostics_export_receipt_failed".to_string())?
            .len();
        Ok(DiagnosticsExportReceipt {
            path: path.to_path_buf(),
            archive_bytes,
            inventory,
            manifest_schema_version: 1,
        })
    }

    fn preview_unlocked(&self, files: &[PathBuf]) -> Result<DiagnosticsPreview, String> {
        let mut categories = BTreeSet::new();
        let mut approximate_bytes = 0_u64;
        let mut first_timestamp: Option<String> = None;
        let mut last_timestamp: Option<String> = None;
        let mut skipped_records = 0;
        for path in files {
            approximate_bytes = approximate_bytes.saturating_add(
                std::fs::metadata(path)
                    .map_err(|_| "diagnostics_preview_failed".to_string())?
                    .len(),
            );
            let file = File::open(path).map_err(|_| "diagnostics_preview_failed".to_string())?;
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if is_event_file(path) {
                    match serde_json::from_str::<StoredDiagnosticEvent>(&line) {
                        Ok(event) => {
                            categories.insert(kind_name(event.kind).to_string());
                            let timestamp = event.timestamp;
                            first_timestamp = Some(first_timestamp.map_or_else(
                                || timestamp.clone(),
                                |current| current.min(timestamp.clone()),
                            ));
                            last_timestamp = Some(last_timestamp.map_or_else(
                                || timestamp.clone(),
                                |current| current.max(timestamp.clone()),
                            ));
                        }
                        Err(_) => skipped_records += 1,
                    }
                }
            }
        }
        Ok(DiagnosticsPreview {
            categories: categories.into_iter().collect(),
            excluded_categories: excluded_categories(),
            file_count: files.len(),
            approximate_bytes,
            app_version: self.app_version.clone(),
            first_timestamp,
            last_timestamp,
            skipped_records,
        })
    }

    fn write_archive(
        &self,
        temporary: &Path,
        files: &[PathBuf],
        preview: &DiagnosticsPreview,
        inventory: &[String],
        private_roots: &[PathBuf],
    ) -> Result<(), String> {
        let file =
            File::create(temporary).map_err(|_| "diagnostics_export_create_failed".to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let manifest = ExportManifest {
            schema_version: 1,
            generated_at: chrono::Utc::now().to_rfc3339(),
            app_version: &self.app_version,
            os_version: &self.os_version,
            categories: &preview.categories,
            excluded_categories: &preview.excluded_categories,
            inventory,
            first_timestamp: &preview.first_timestamp,
            last_timestamp: &preview.last_timestamp,
            skipped_records: preview.skipped_records,
        };
        zip.start_file("manifest.json", options)
            .map_err(|_| "diagnostics_export_write_failed".to_string())?;
        zip.write_all(
            &serde_json::to_vec_pretty(&manifest)
                .map_err(|_| "diagnostics_export_write_failed".to_string())?,
        )
        .map_err(|_| "diagnostics_export_write_failed".to_string())?;

        for (index, source) in files.iter().enumerate() {
            zip.start_file(archive_name(source, index), options)
                .map_err(|_| "diagnostics_export_write_failed".to_string())?;
            let file =
                File::open(source).map_err(|_| "diagnostics_export_read_failed".to_string())?;
            for line in BufReader::new(file).lines() {
                let line = line.map_err(|_| "diagnostics_export_read_failed".to_string())?;
                if is_event_file(source)
                    && serde_json::from_str::<StoredDiagnosticEvent>(&line).is_err()
                {
                    continue;
                }
                let sanitized = sanitize_message(&line, private_roots);
                zip.write_all(sanitized.as_bytes())
                    .and_then(|_| zip.write_all(b"\n"))
                    .map_err(|_| "diagnostics_export_write_failed".to_string())?;
            }
        }
        let output = zip
            .finish()
            .map_err(|_| "diagnostics_export_finish_failed".to_string())?;
        output
            .sync_all()
            .map_err(|_| "diagnostics_export_finish_failed".to_string())
    }

    fn rotate_if_needed(&self, incoming_bytes: u64) -> Result<(), String> {
        let current = self.root.join("events.ndjson");
        let current_bytes = std::fs::metadata(&current)
            .map(|meta| meta.len())
            .unwrap_or(0);
        let per_file_limit = (self.max_total_bytes / self.max_files.max(1) as u64).max(1);
        if current_bytes.saturating_add(incoming_bytes) <= per_file_limit {
            return Ok(());
        }
        if self.max_files <= 1 {
            let _ = std::fs::remove_file(current);
            return Ok(());
        }
        let oldest = self
            .root
            .join(format!("events.{}.ndjson", self.max_files - 1));
        let _ = std::fs::remove_file(oldest);
        for index in (1..self.max_files - 1).rev() {
            let source = self.root.join(format!("events.{index}.ndjson"));
            let target = self.root.join(format!("events.{}.ndjson", index + 1));
            if source.exists() {
                std::fs::rename(source, target)
                    .map_err(|_| "diagnostics_rotation_failed".to_string())?;
            }
        }
        if current.exists() {
            std::fs::rename(current, self.root.join("events.1.ndjson"))
                .map_err(|_| "diagnostics_rotation_failed".to_string())?;
        }
        Ok(())
    }

    fn enforce_total_size(&self) -> Result<(), String> {
        let mut files = self.event_files()?;
        while files.len() > self.max_files
            || files
                .iter()
                .filter_map(|path| std::fs::metadata(path).ok())
                .map(|meta| meta.len())
                .sum::<u64>()
                > self.max_total_bytes
        {
            let Some(oldest) = files.first().cloned() else {
                break;
            };
            std::fs::remove_file(oldest).map_err(|_| "diagnostics_retention_failed".to_string())?;
            files = self.event_files()?;
        }
        Ok(())
    }

    fn event_files(&self) -> Result<Vec<PathBuf>, String> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err("diagnostics_directory_read_failed".to_string()),
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name == "events.ndjson"
                            || (name.starts_with("events.") && name.ends_with(".ndjson"))
                    })
            })
            .collect();
        files.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
        Ok(files)
    }

    fn support_files(&self) -> Result<Vec<PathBuf>, String> {
        let mut files = self.event_files()?;
        let logs = self.root.join("logs");
        match std::fs::read_dir(logs) {
            Ok(entries) => files.extend(
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| path.is_file()),
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("diagnostics_directory_read_failed".to_string()),
        }
        files.sort();
        Ok(files)
    }
}

#[tauri::command]
pub fn diagnostics_record_renderer_error(
    service: State<'_, Arc<DiagnosticsService>>,
    input: RendererDiagnosticInput,
) -> Result<(), String> {
    if !matches!(
        input.transition.as_str(),
        "window_error" | "unhandled_rejection"
    ) {
        return Err("diagnostics_renderer_transition_invalid".to_string());
    }
    if !matches!(
        input.message_code.as_str(),
        "renderer_window_error" | "renderer_unhandled_rejection"
    ) || !input.error_class.chars().enumerate().all(|(index, ch)| {
        (index == 0 && ch.is_ascii_alphabetic())
            || (index > 0 && (ch.is_ascii_alphanumeric() || ch == '_'))
    }) || input.error_class.len() > 64
    {
        return Err("diagnostics_renderer_payload_invalid".to_string());
    }
    service.record(
        DiagnosticEventInput {
            level: DiagnosticLevel::Error,
            kind: DiagnosticKind::RendererError,
            transition: Some(&input.transition),
            error_class: Some(&input.error_class),
            message_code: Some(&input.message_code),
        },
        &default_private_roots(),
    )
}

#[tauri::command]
pub fn diagnostics_preview_export(
    service: State<'_, Arc<DiagnosticsService>>,
) -> Result<DiagnosticsPreview, String> {
    service.preview()
}

#[tauri::command]
pub fn diagnostics_export(
    service: State<'_, Arc<DiagnosticsService>>,
    path: PathBuf,
) -> Result<DiagnosticsExportReceipt, String> {
    if path.is_dir() {
        return Err("diagnostics_export_path_is_directory".to_string());
    }
    let path = if path.extension().and_then(|value| value.to_str()) == Some("zip") {
        path
    } else {
        path.with_extension("zip")
    };
    service.export(&path, &default_private_roots())
}

pub fn build_log_plugin<R: Runtime>(root: PathBuf) -> tauri::plugin::TauriPlugin<R> {
    let private_roots = default_private_roots();
    let target = tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Folder {
        path: root.join("logs"),
        file_name: Some("beefex".to_string()),
    })
    .filter(|metadata| metadata.target() == LOG_TARGET)
    .format(move |out, message, record| {
        let sanitized = sanitize_message(&message.to_string(), &private_roots);
        out.finish(format_args!(
            "[{}][{}] {}",
            record.level(),
            record.target(),
            sanitized
        ))
    });
    tauri_plugin_log::Builder::new()
        .clear_targets()
        .level(log::LevelFilter::Info)
        .max_file_size(512 * 1024)
        .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(2))
        .target(target)
        .build()
}

pub fn install_panic_hook(service: Arc<DiagnosticsService>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = info;
        let _ = service.record(
            DiagnosticEventInput {
                level: DiagnosticLevel::Error,
                kind: DiagnosticKind::RustPanic,
                transition: Some("panic"),
                error_class: Some("rust_panic"),
                message_code: Some("rust_panic_captured"),
            },
            &default_private_roots(),
        );
        previous(info);
    }));
}

pub fn cleanup_existing_store(root: &Path) -> Result<(), String> {
    let mut files: Vec<(PathBuf, std::time::SystemTime, u64)> = match std::fs::read_dir(root) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .flat_map(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    std::fs::read_dir(path)
                        .into_iter()
                        .flatten()
                        .filter_map(Result::ok)
                        .map(|entry| entry.path())
                        .collect::<Vec<_>>()
                } else {
                    vec![path]
                }
            })
            .filter_map(|path| {
                let metadata = std::fs::metadata(&path).ok()?;
                metadata.is_file().then(|| {
                    (
                        path,
                        metadata
                            .modified()
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                        metadata.len(),
                    )
                })
            })
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("diagnostics_directory_read_failed".to_string()),
    };
    files.sort_by_key(|(_, modified, _)| *modified);
    let mut total_bytes = files.iter().map(|(_, _, bytes)| *bytes).sum::<u64>();
    while files.len() > STORE_MAX_FILES || total_bytes > STORE_MAX_TOTAL_BYTES {
        let (path, _, bytes) = files.remove(0);
        std::fs::remove_file(path).map_err(|_| "diagnostics_retention_failed".to_string())?;
        total_bytes = total_bytes.saturating_sub(bytes);
    }
    Ok(())
}

pub fn default_private_roots() -> Vec<PathBuf> {
    ["HOME", "TMPDIR"]
        .into_iter()
        .filter_map(|key| std::env::var_os(key).map(PathBuf::from))
        .collect()
}

pub fn platform_version() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
        {
            if output.status.success() {
                let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !version.is_empty() {
                    return format!("macOS {version}");
                }
            }
        }
    }
    std::env::consts::OS.to_string()
}

pub fn record_app_event(
    app: &tauri::AppHandle,
    kind: DiagnosticKind,
    level: DiagnosticLevel,
    transition: &str,
    error_class: Option<&str>,
    message_code: Option<&str>,
    private_roots: &[PathBuf],
) {
    if let Some(service) = app.try_state::<Arc<DiagnosticsService>>() {
        let _ = service.record(
            DiagnosticEventInput {
                level,
                kind,
                transition: Some(transition),
                error_class,
                message_code,
            },
            private_roots,
        );
    }
}

fn archive_name(path: &Path, index: usize) -> String {
    if is_event_file(path) {
        format!("events/events-{index:02}.ndjson")
    } else {
        format!("logs/log-{index:02}.txt")
    }
}

fn is_event_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name == "events.ndjson" || (name.starts_with("events.") && name.ends_with(".ndjson"))
        })
}

fn kind_name(kind: DiagnosticKind) -> &'static str {
    match kind {
        DiagnosticKind::Startup => "startup",
        DiagnosticKind::AccountTransition => "account_transition",
        DiagnosticKind::PiChildLifecycle => "pi_child_lifecycle",
        DiagnosticKind::RunTerminal => "run_terminal",
        DiagnosticKind::TaskRecovery => "task_recovery",
        DiagnosticKind::RendererError => "renderer_error",
        DiagnosticKind::RustPanic => "rust_panic",
        DiagnosticKind::ExportLifecycle => "export_lifecycle",
    }
}

fn excluded_categories() -> Vec<String> {
    [
        "credentials",
        "account_identifiers",
        "prompts_and_transcripts",
        "tool_payloads",
        "project_content",
        "absolute_paths",
        "request_and_response_bodies",
        "raw_pi_events",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn credential_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r"(?i)(authorization\s*:\s*bearer\s+|bearer\s+|api[_-]?key\s*[=:]\s*|password\s*[=:]\s*)[^\s,;]+",
        )
        .expect("credential redaction regex")
    })
}

fn email_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?i)\b[a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9.-]+\.[a-z]{2,}\b")
            .expect("email redaction regex")
    })
}

fn url_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(https?://[^\s?#]+)(?:\?[^\s#]*)?(?:#[^\s]*)?").expect("url redaction regex")
    })
}

fn unix_absolute_path_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r#"(?P<lead>^|[\s=\"'(])/(?:[^/\s]+/)+[^/\s,;)\]}]+"#)
            .expect("unix path redaction regex")
    })
}

fn windows_absolute_path_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r#"(?i)(?P<lead>^|[\s=\"'(])[a-z]:\\(?:[^\\\s]+\\)*[^\\\s,;)\]}]+"#)
            .expect("windows path redaction regex")
    })
}

fn sensitive_payload_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r#"(?i)(prompt|transcript|tool[_-]?(?:args?|result)|raw[_-]?pi[_-]?event)\s*[:=]\s*(?:\"[^\"]*\"|[^\s,;]+)"#)
            .expect("sensitive payload regex")
    })
}

pub fn sanitize_message(input: &str, private_roots: &[PathBuf]) -> String {
    let mut output = url_pattern().replace_all(input, "$1").into_owned();
    output = credential_pattern()
        .replace_all(&output, "${1}[REDACTED_CREDENTIAL]")
        .into_owned();
    output = email_pattern()
        .replace_all(&output, "[REDACTED_EMAIL]")
        .into_owned();
    output = sensitive_payload_pattern()
        .replace_all(&output, "${1}=[REDACTED_PAYLOAD]")
        .into_owned();
    output = unix_absolute_path_pattern()
        .replace_all(&output, "${lead}[PRIVATE_PATH]")
        .into_owned();
    output = windows_absolute_path_pattern()
        .replace_all(&output, "${lead}[PRIVATE_PATH]")
        .into_owned();

    let mut roots: Vec<String> = private_roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .filter(|root| !root.is_empty())
        .collect();
    roots.sort_by_key(|root| std::cmp::Reverse(root.len()));
    roots.dedup();
    for root in roots {
        output = output.replace(&root, "[PRIVATE_PATH]");
    }

    if output.len() > MAX_MESSAGE_BYTES {
        let mut end = MAX_MESSAGE_BYTES;
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
        output.push_str("[TRUNCATED]");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "beefex-diagnostics-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("create diagnostics fixture dir");
        path
    }

    #[test]
    fn sanitizes_forbidden_values_before_persistence() {
        let home = PathBuf::from("/Users/tester");
        let project = home.join("Work/secret-project");
        let input = format!(
            "Authorization: Bearer sk-live-secret email=lucas@example.com path={} external=/Volumes/External/project/file.rs url=https://example.com/run?token=secret#trace",
            project.display()
        );

        let sanitized = sanitize_message(&input, &[home, project]);

        for forbidden in [
            "sk-live-secret",
            "lucas@example.com",
            "/Users/tester",
            "/Volumes/External/project/file.rs",
            "token=secret",
            "#trace",
        ] {
            assert!(
                !sanitized.contains(forbidden),
                "leaked {forbidden}: {sanitized}"
            );
        }
        assert!(sanitized.contains("[REDACTED_CREDENTIAL]"));
        assert!(sanitized.contains("[REDACTED_EMAIL]"));
        assert!(sanitized.contains("[PRIVATE_PATH]"));
        assert!(sanitized.contains("https://example.com/run"));

        let payloads = sanitize_message(
            r#"prompt=secret transcript:\"private chat\" tool_args={secret} tool_result=private raw_pi_event=payload"#,
            &[],
        );
        for forbidden in ["secret", "private chat", "{secret}", "private", "payload"] {
            assert!(
                !payloads.contains(forbidden),
                "leaked payload {forbidden}: {payloads}"
            );
        }
        assert!(payloads.contains("[REDACTED_PAYLOAD]"));
        assert!(sanitize_message(&"测".repeat(600), &[]).len() <= MAX_MESSAGE_BYTES + 11);
    }

    #[test]
    fn records_previews_and_exports_a_secret_free_archive() {
        let root = temp_dir("export");
        let private_root = root.join("secret-project");
        let service = DiagnosticsService::new(root.join("store"), "0.1.0", "macOS 15");
        let message = format!(
            "Bearer sk-fixture-secret lucas@example.com {} https://example.com/fail?token=secret",
            private_root.display()
        );
        service
            .record(
                DiagnosticEventInput {
                    level: DiagnosticLevel::Error,
                    kind: DiagnosticKind::RendererError,
                    transition: Some("window_error"),
                    error_class: Some("TypeError"),
                    message_code: Some(&message),
                },
                &[private_root.clone()],
            )
            .expect("record diagnostic event");

        let preview = service.preview().expect("preview diagnostics");
        assert_eq!(preview.file_count, 1);
        assert!(preview.approximate_bytes > 0);
        assert!(preview.categories.contains(&"renderer_error".to_string()));
        assert!(preview
            .excluded_categories
            .contains(&"credentials".to_string()));

        let archive_path = root.join("support.zip");
        let receipt = service
            .export(&archive_path, &[private_root.clone()])
            .expect("export diagnostics");
        assert_eq!(receipt.manifest_schema_version, 1);
        assert!(receipt.inventory.contains(&"manifest.json".to_string()));

        let file = std::fs::File::open(&archive_path).expect("open support archive");
        let mut archive = zip::ZipArchive::new(file).expect("read support archive");
        let mut combined = String::new();
        for index in 0..archive.len() {
            archive
                .by_index(index)
                .expect("archive member")
                .read_to_string(&mut combined)
                .expect("read archive member");
        }
        for forbidden in [
            "sk-fixture-secret",
            "lucas@example.com",
            private_root.to_string_lossy().as_ref(),
            "token=secret",
        ] {
            assert!(!combined.contains(forbidden), "archive leaked {forbidden}");
        }

        std::fs::remove_dir_all(root).expect("remove diagnostics fixture");
    }

    #[test]
    fn retention_keeps_the_newest_events_within_file_and_byte_limits() {
        let root = temp_dir("retention");
        let service =
            DiagnosticsService::new(root.join("store"), "0.1.0", "macOS 15").with_limits(3, 1_200);
        for index in 0..20 {
            let message_code = format!("event-{index:02}-{}", "x".repeat(80));
            service
                .record(
                    DiagnosticEventInput {
                        level: DiagnosticLevel::Info,
                        kind: DiagnosticKind::PiChildLifecycle,
                        transition: Some("event"),
                        error_class: None,
                        message_code: Some(&message_code),
                    },
                    &[],
                )
                .expect("record retained event");
        }

        let files = service.event_files().expect("list retained files");
        let total_bytes: u64 = files
            .iter()
            .map(|path| std::fs::metadata(path).expect("retained metadata").len())
            .sum();
        assert!(files.len() <= 3, "retained {} files", files.len());
        assert!(total_bytes <= 1_200, "retained {total_bytes} bytes");
        let combined = files
            .iter()
            .map(|path| std::fs::read_to_string(path).expect("read retained file"))
            .collect::<String>();
        assert!(combined.contains("event-19-"), "newest event was evicted");

        std::fs::remove_dir_all(root).expect("remove retention fixture");
    }

    #[test]
    fn corrupt_lines_are_counted_and_skipped_from_export() {
        let root = temp_dir("corrupt");
        let store = root.join("store");
        let service = DiagnosticsService::new(store.clone(), "0.1.0", "macOS 15");
        service
            .record(
                DiagnosticEventInput {
                    level: DiagnosticLevel::Info,
                    kind: DiagnosticKind::Startup,
                    transition: Some("ready"),
                    error_class: None,
                    message_code: Some("app_ready"),
                },
                &[],
            )
            .expect("record fixture event");
        let mut event_file = OpenOptions::new()
            .append(true)
            .open(store.join("events.ndjson"))
            .expect("open event file");
        writeln!(event_file, "corrupt raw line with seeded-secret").expect("append corrupt line");

        let preview = service.preview().expect("preview corrupt fixture");
        assert_eq!(preview.skipped_records, 1);
        assert!(preview.first_timestamp.is_some());
        assert!(preview.last_timestamp.is_some());

        let archive_path = root.join("support.zip");
        service
            .export(&archive_path, &[])
            .expect("export corrupt fixture");
        let file = File::open(archive_path).expect("open archive");
        let mut archive = zip::ZipArchive::new(file).expect("read archive");
        let mut combined = String::new();
        for index in 0..archive.len() {
            archive
                .by_index(index)
                .expect("archive member")
                .read_to_string(&mut combined)
                .expect("read archive member");
        }
        assert!(!combined.contains("seeded-secret"));
        assert!(combined.contains("\"skippedRecords\": 1"));
        std::fs::remove_dir_all(root).expect("remove corrupt fixture");
    }

    #[test]
    fn failed_atomic_replace_removes_temporary_archive() {
        let root = temp_dir("atomic-failure");
        let service = DiagnosticsService::new(root.join("store"), "0.1.0", "macOS 15");
        service
            .record(
                DiagnosticEventInput {
                    level: DiagnosticLevel::Info,
                    kind: DiagnosticKind::Startup,
                    transition: Some("ready"),
                    error_class: None,
                    message_code: Some("app_ready"),
                },
                &[],
            )
            .expect("record fixture event");
        let destination = root.join("destination.zip");
        std::fs::create_dir(&destination).expect("create conflicting destination");
        assert_eq!(
            service
                .export(&destination, &[])
                .expect_err("replace must fail"),
            "diagnostics_export_replace_failed"
        );
        let temporary_files = std::fs::read_dir(&root)
            .expect("list fixture root")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("tmp-"))
            .count();
        assert_eq!(temporary_files, 0);
        std::fs::remove_dir_all(root).expect("remove atomic fixture");
    }

    #[test]
    fn startup_cleanup_enforces_combined_file_and_byte_caps() {
        let root = temp_dir("startup-cleanup");
        for index in 0..9 {
            let path = root.join(format!("diagnostic-{index}.log"));
            let file = File::create(path).expect("create retained fixture");
            file.set_len(3 * 1024 * 1024)
                .expect("size retained fixture");
        }
        cleanup_existing_store(&root).expect("cleanup existing diagnostics");
        let files: Vec<PathBuf> = std::fs::read_dir(&root)
            .expect("list retained fixtures")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        let total = files
            .iter()
            .map(|path| std::fs::metadata(path).expect("fixture metadata").len())
            .sum::<u64>();
        assert!(files.len() <= STORE_MAX_FILES);
        assert!(total <= STORE_MAX_TOTAL_BYTES);
        std::fs::remove_dir_all(root).expect("remove cleanup fixture");
    }

    #[test]
    fn offline_lifecycle_fixture_exports_only_closed_event_categories() {
        let root = temp_dir("offline-lifecycle");
        let service = DiagnosticsService::new(root.join("store"), "0.1.0", "macOS 15");
        let events = [
            (DiagnosticKind::Startup, "setup_ready", "app_setup_ready"),
            (
                DiagnosticKind::AccountTransition,
                "offline",
                "network_unavailable",
            ),
            (
                DiagnosticKind::PiChildLifecycle,
                "spawned",
                "pi_child_spawned",
            ),
            (DiagnosticKind::PiChildLifecycle, "eof", "pi_child_eof"),
            (DiagnosticKind::RunTerminal, "failed", "error"),
            (
                DiagnosticKind::TaskRecovery,
                "recovered",
                "pi_session_recovered",
            ),
        ];
        for (kind, transition, message_code) in events {
            service
                .record(
                    DiagnosticEventInput {
                        level: DiagnosticLevel::Info,
                        kind,
                        transition: Some(transition),
                        error_class: None,
                        message_code: Some(message_code),
                    },
                    &[],
                )
                .expect("record offline lifecycle event");
        }
        let preview = service.preview().expect("preview offline lifecycle");
        for category in [
            "startup",
            "account_transition",
            "pi_child_lifecycle",
            "run_terminal",
            "task_recovery",
        ] {
            assert!(preview.categories.contains(&category.to_string()));
        }
        let archive = root.join("offline.zip");
        let receipt = service
            .export(&archive, &[])
            .expect("export offline lifecycle");
        assert!(receipt.archive_bytes > 0);
        assert_eq!(
            receipt.inventory.first().map(String::as_str),
            Some("manifest.json")
        );
        std::fs::remove_dir_all(root).expect("remove offline fixture");
    }
}
