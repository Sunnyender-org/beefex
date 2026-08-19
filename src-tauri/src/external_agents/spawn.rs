use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;

use crate::external_agents::types::{PromptInputFormat, RuntimeAgentDef};
use crate::proc::NoConsoleWindow;

pub struct SpawnedAgent {
    pub child: Child,
    pub resolved_bin: PathBuf,
}

/// Bun standalone executables on Windows 11 can crash while loading TypeScript extensions when
/// a Win32 verbatim path (`\\?\C:\...` / `\\?\UNC\...`) crosses the process boundary. Keep the
/// canonical/verbatim paths inside Beefex for filesystem safety, but present ordinary absolute
/// Win32 paths to the Pi child. This is a lexical conversion only; it does not resolve symlinks or
/// change which file the already-validated path names.
fn normalize_windows_verbatim_path(value: &str) -> String {
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        value.to_string()
    }
}

fn pi_process_boundary_path(path: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        return PathBuf::from(normalize_windows_verbatim_path(&path.to_string_lossy()));
    }
    #[cfg(not(target_os = "windows"))]
    {
        path.to_path_buf()
    }
}

fn pi_process_boundary_value(value: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        return normalize_windows_verbatim_path(value);
    }
    #[cfg(not(target_os = "windows"))]
    {
        value.to_string()
    }
}

/// Concurrently drain the child's stderr into a JoinHandle so a CLI that reports failures on
/// stderr doesn't (a) block on a full pipe while we read stdout, and (b) fail silently. Blank
/// lines are dropped and the buffer is capped at `STDERR_CAP_CHARS` (keeping the tail — the last
/// lines are usually the actual error). Call before the stdout read loop; await after `wait()`.
pub fn drain_stderr(child: &mut Child) -> tokio::task::JoinHandle<String> {
    const STDERR_CAP_CHARS: usize = 8192;
    let stderr = child.stderr.take();
    tokio::spawn(async move {
        let Some(stderr) = stderr else {
            return String::new();
        };
        let mut reader = BufReader::new(stderr).lines();
        let mut out = String::new();
        while let Ok(Some(line)) = reader.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&line);
            if out.chars().count() > STDERR_CAP_CHARS {
                out = tail_chars(&out, STDERR_CAP_CHARS);
            }
        }
        out
    })
}

/// Keep the last `max_chars` characters of `value` (char-boundary safe).
pub fn tail_chars(value: &str, max_chars: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    let start = chars.len().saturating_sub(max_chars);
    chars[start..].iter().collect()
}

pub async fn resolve_binary(def: &RuntimeAgentDef) -> Option<PathBuf> {
    if def.id == "pi" {
        if let Some(path) = resolve_bundled_pi_binary() {
            return Some(path);
        }
        if !cfg!(debug_assertions) {
            return None;
        }
    }
    for candidate in std::iter::once(def.bin).chain(def.fallback_bins.iter().copied()) {
        if let Some(path) = which_binary(candidate).await {
            return Some(path);
        }
    }
    None
}

fn resolve_bundled_pi_binary() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::PermissionsExt;

        fn is_executable_file(path: &Path) -> bool {
            let metadata = match std::fs::metadata(path) {
                Ok(metadata) => metadata,
                Err(_) => return false,
            };
            metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
        }

        let executable = std::env::current_exe().ok()?;
        let packaged = executable.parent()?.join("../Resources/pi/bin/pi");
        if is_executable_file(&packaged) {
            return Some(packaged);
        }

        if cfg!(debug_assertions) {
            let development =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../node_modules/.bin/pi");
            if is_executable_file(&development) {
                return Some(development);
            }
        }

        None
    }

    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(target_os = "windows")]
        {
            let executable = std::env::current_exe().ok()?;
            let packaged = executable.parent()?.join("pi/bin/pi.exe");
            if packaged.is_file() {
                return Some(packaged);
            }

            if cfg!(debug_assertions) {
                let development =
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/pi/bin/pi.exe");
                if development.is_file() {
                    return Some(development);
                }
            }
        }

        None
    }
}

async fn which_binary(name: &str) -> Option<PathBuf> {
    let output = Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg(name)
        .no_console_window()
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    if line.is_empty() {
        None
    } else {
        Some(PathBuf::from(line))
    }
}

pub async fn spawn_agent(
    def: &RuntimeAgentDef,
    resolved_bin: &Path,
    args: &[String],
    cwd: &Path,
    extra_env: &HashMap<String, String>,
) -> Result<SpawnedAgent, String> {
    let process_bin = if def.id == "pi" {
        pi_process_boundary_path(resolved_bin)
    } else {
        resolved_bin.to_path_buf()
    };
    let process_cwd = if def.id == "pi" {
        pi_process_boundary_path(cwd)
    } else {
        cwd.to_path_buf()
    };
    let process_args = if def.id == "pi" {
        args.iter()
            .map(|value| pi_process_boundary_value(value))
            .collect::<Vec<_>>()
    } else {
        args.to_vec()
    };

    let mut command = Command::new(&process_bin);
    if def.id == "pi" {
        // Managed Pi must never inherit ambient provider credentials. The caller supplies a
        // minimal process environment plus the parent-owned broker/session paths explicitly.
        command.env_clear();
    }
    command
        .args(&process_args)
        .current_dir(&process_cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .no_console_window()
        .kill_on_drop(true);
    for (key, value) in def.env {
        command.env(key, value);
    }
    for (key, value) in extra_env {
        if def.id == "pi" {
            command.env(key, pi_process_boundary_value(value));
        } else {
            command.env(key, value);
        }
    }
    let child = command
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", def.id))?;
    Ok(SpawnedAgent {
        child,
        resolved_bin: resolved_bin.to_path_buf(),
    })
}

#[cfg(test)]
mod process_boundary_tests {
    use super::normalize_windows_verbatim_path;

    #[test]
    fn strips_windows_drive_verbatim_prefix() {
        assert_eq!(
            normalize_windows_verbatim_path(r"\\?\C:\Beefex\pi\bin\pi.exe"),
            r"C:\Beefex\pi\bin\pi.exe"
        );
    }

    #[test]
    fn converts_windows_unc_verbatim_prefix() {
        assert_eq!(
            normalize_windows_verbatim_path(r"\\?\UNC\server\share\pi.exe"),
            r"\\server\share\pi.exe"
        );
    }

    #[test]
    fn preserves_ordinary_paths_and_non_path_values() {
        assert_eq!(
            normalize_windows_verbatim_path(r"C:\Beefex\pi\bin\pi.exe"),
            r"C:\Beefex\pi\bin\pi.exe"
        );
        assert_eq!(
            normalize_windows_verbatim_path("gpt-5.6-sol"),
            "gpt-5.6-sol"
        );
    }
}

pub async fn write_prompt_stdin(
    child: &mut Child,
    def: &RuntimeAgentDef,
    prompt: &str,
) -> Result<(), String> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "stdin unavailable".to_string())?;
    let mut stdin = stdin;
    match def.prompt_input_format {
        PromptInputFormat::Text => {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            stdin.shutdown().await.map_err(|e| e.to_string())?;
        }
        PromptInputFormat::StreamJson => {
            let content = stream_json_user_content(prompt);
            let line = serde_json::json!({
                "type": "user",
                "message": {
                    "role": "user",
                    "content": content
                },
                "parent_tool_use_id": null
            });
            let mut payload = serde_json::to_string(&line).map_err(|e| e.to_string())?;
            payload.push('\n');
            stdin
                .write_all(payload.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Minimal stdin write to elicit Claude `system/init` during slash-command probing.
pub async fn write_probe_stdin(child: &mut Child) -> Result<(), String> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "stdin unavailable".to_string())?;
    let mut stdin = stdin;
    let line = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": "."
        },
        "parent_tool_use_id": null
    });
    let mut payload = serde_json::to_string(&line).map_err(|e| e.to_string())?;
    payload.push('\n');
    stdin
        .write_all(payload.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn stream_json_user_content(prompt: &str) -> serde_json::Value {
    if prompt.trim_start().starts_with('/') {
        serde_json::Value::String(prompt.to_string())
    } else {
        serde_json::json!([{ "type": "text", "text": prompt }])
    }
}

pub async fn read_stdout_lines<F>(
    child: &mut Child,
    mut on_line: F,
    cancel_check: impl Fn() -> bool,
) -> Result<(), String>
where
    F: FnMut(&str) -> Result<(), String>,
{
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout unavailable".to_string())?;
    let mut reader = BufReader::new(stdout).lines();
    loop {
        if cancel_check() {
            let _ = child.start_kill();
            return Err("cancelled".to_string());
        }
        let line = match timeout(Duration::from_millis(200), reader.next_line()).await {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => break,
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        on_line(&line)?;
    }
    Ok(())
}

pub fn parse_json_line(line: &str) -> Option<serde_json::Value> {
    serde_json::from_str(line.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_pi_resolution_points_to_the_pinned_runtime() {
        let pi = resolve_bundled_pi_binary().expect("bundled/development Pi runtime");
        #[cfg(target_os = "windows")]
        assert_eq!(
            pi.file_name().and_then(|name| name.to_str()),
            Some("pi.exe")
        );

        let output = std::process::Command::new(&pi)
            .arg("--version")
            .output()
            .expect("execute resolved Pi runtime");
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "0.84.1");
    }

    #[test]
    fn stream_json_user_content_uses_string_for_slash_commands() {
        let slash = stream_json_user_content("/compact");
        assert_eq!(slash, serde_json::json!("/compact"));
        let text = stream_json_user_content("hello");
        assert_eq!(
            text,
            serde_json::json!([{ "type": "text", "text": "hello" }])
        );
    }
}
