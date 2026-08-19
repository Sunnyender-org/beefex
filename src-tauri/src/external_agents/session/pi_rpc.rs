use std::future::Future;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

use crate::beefapi::pi_broker::PiProviderBroker;
use crate::external_agents::context::parse_context_window_label;
use crate::external_agents::session::live::SessionCommand;
use crate::external_agents::spawn::drain_stderr;
use crate::external_agents::stream::usage_from_numbers;
use crate::external_agents::types::{
    default_model_option, ExternalCliSlashCommand, RuntimeModelOption, UnifiedAgentEvent,
};
use crate::proc::NoConsoleWindow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiRpcOutcome {
    Continue,
    AgentEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiRpcSessionState {
    pub session_file: String,
    pub session_id: String,
    pub session_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiExtensionUiRequest {
    pub id: String,
    pub method: String,
    pub title: String,
    pub message: String,
    pub placeholder: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PiExtensionUiDecision {
    Confirmed(bool),
    Value(String),
    Cancelled,
}

pub struct PiExtensionUiExchange {
    pub request: PiExtensionUiRequest,
    pub response: oneshot::Sender<PiExtensionUiDecision>,
}

/// Complete command surface of the pinned Pi 0.84.1 RPC protocol. Desktop callers must still
/// apply product policy (for example BeefAPI-only models) before dispatching a command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PiRpcCommand {
    Prompt {
        message: String,
    },
    Steer {
        message: String,
    },
    FollowUp {
        message: String,
    },
    Abort,
    NewSession {
        parent_session: Option<String>,
    },
    GetState,
    SetModel {
        provider: String,
        model_id: String,
    },
    CycleModel,
    GetAvailableModels,
    SetThinkingLevel {
        level: String,
    },
    CycleThinkingLevel,
    GetAvailableThinkingLevels,
    SetSteeringMode {
        mode: String,
    },
    SetFollowUpMode {
        mode: String,
    },
    Compact {
        custom_instructions: Option<String>,
    },
    SetAutoCompaction {
        enabled: bool,
    },
    SetAutoRetry {
        enabled: bool,
    },
    AbortRetry,
    Bash {
        command: String,
        exclude_from_context: Option<bool>,
    },
    AbortBash,
    GetSessionStats,
    ExportHtml {
        output_path: Option<String>,
    },
    SwitchSession {
        session_path: String,
    },
    Fork {
        entry_id: String,
    },
    Clone,
    GetForkMessages,
    GetEntries {
        since: Option<String>,
    },
    GetTree,
    GetLastAssistantText,
    SetSessionName {
        name: String,
    },
    GetMessages,
    GetCommands,
}

impl PiRpcCommand {
    pub fn command_name(&self) -> &'static str {
        match self {
            Self::Prompt { .. } => "prompt",
            Self::Steer { .. } => "steer",
            Self::FollowUp { .. } => "follow_up",
            Self::Abort => "abort",
            Self::NewSession { .. } => "new_session",
            Self::GetState => "get_state",
            Self::SetModel { .. } => "set_model",
            Self::CycleModel => "cycle_model",
            Self::GetAvailableModels => "get_available_models",
            Self::SetThinkingLevel { .. } => "set_thinking_level",
            Self::CycleThinkingLevel => "cycle_thinking_level",
            Self::GetAvailableThinkingLevels => "get_available_thinking_levels",
            Self::SetSteeringMode { .. } => "set_steering_mode",
            Self::SetFollowUpMode { .. } => "set_follow_up_mode",
            Self::Compact { .. } => "compact",
            Self::SetAutoCompaction { .. } => "set_auto_compaction",
            Self::SetAutoRetry { .. } => "set_auto_retry",
            Self::AbortRetry => "abort_retry",
            Self::Bash { .. } => "bash",
            Self::AbortBash => "abort_bash",
            Self::GetSessionStats => "get_session_stats",
            Self::ExportHtml { .. } => "export_html",
            Self::SwitchSession { .. } => "switch_session",
            Self::Fork { .. } => "fork",
            Self::Clone => "clone",
            Self::GetForkMessages => "get_fork_messages",
            Self::GetEntries { .. } => "get_entries",
            Self::GetTree => "get_tree",
            Self::GetLastAssistantText => "get_last_assistant_text",
            Self::SetSessionName { .. } => "set_session_name",
            Self::GetMessages => "get_messages",
            Self::GetCommands => "get_commands",
        }
    }

    pub fn changes_session(&self) -> bool {
        matches!(
            self,
            Self::NewSession { .. } | Self::SwitchSession { .. } | Self::Fork { .. } | Self::Clone
        )
    }

    fn with_id(&self, id: String) -> Result<Value, String> {
        let mut value = serde_json::to_value(self).map_err(|error| error.to_string())?;
        value
            .as_object_mut()
            .ok_or_else(|| "Pi RPC command must serialize to an object".to_string())?
            .insert("id".to_string(), Value::String(id));
        Ok(value)
    }
}

/// Discover Pi slash commands via the RPC `get_commands` request.
/// Response shape: `{type:"response", command:"get_commands", data:{commands:[{name, description}]}}`.
pub async fn detect_pi_commands(
    bin: &Path,
    args: &[&str],
    cwd: &Path,
    timeout_secs: u64,
) -> Option<Vec<ExternalCliSlashCommand>> {
    let mut child = Command::new(bin)
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .no_console_window()
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let mut reader = BufReader::new(stdout).lines();

    let req = json!({ "id": 1, "type": "get_commands" }).to_string();
    stdin.write_all(format!("{req}\n").as_bytes()).await.ok()?;

    let started = std::time::Instant::now();
    let mut commands: Option<Vec<ExternalCliSlashCommand>> = None;
    loop {
        if started.elapsed() > Duration::from_secs(timeout_secs) {
            break;
        }
        let line = match timeout(Duration::from_millis(200), reader.next_line()).await {
            Ok(Ok(Some(l))) => l,
            Ok(Ok(None)) => break,
            Ok(Err(_)) => break,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let is_get_commands = value.get("type").and_then(|v| v.as_str()) == Some("response")
            && value.get("command").and_then(|v| v.as_str()) == Some("get_commands");
        if !is_get_commands {
            continue;
        }
        let list = value
            .get("data")
            .and_then(|d| d.get("commands"))
            .and_then(|v| v.as_array());
        if let Some(list) = list {
            let mut out = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for raw in list {
                let Some(name) = raw
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                if seen.insert(name.to_string()) {
                    out.push(ExternalCliSlashCommand {
                        slash: format!("/{name}"),
                        name: name.to_string(),
                        description: raw
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(|d| d.trim().to_string())
                            .filter(|d| !d.is_empty()),
                        argument_hint: None,
                    });
                }
            }
            out.sort_by(|a, b| a.name.cmp(&b.name));
            commands = Some(out);
        }
        break;
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
    commands.filter(|c| !c.is_empty())
}

const FIRE_AND_FORGET: &[&str] = &[
    "setStatus",
    "setWidget",
    "notify",
    "setTitle",
    "set_editor_text",
];
const AGENT_END_SETTLE_FALLBACK: Duration = Duration::from_secs(1);

#[derive(Debug, Default)]
struct PiTerminalTracker {
    canonical_settled: bool,
    fallback_agent_end_at: Option<Instant>,
}

impl PiTerminalTracker {
    fn observe(&mut self, value: &Value, now: Instant) {
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match event_type {
            "agent_settled" => {
                self.canonical_settled = true;
                self.fallback_agent_end_at = None;
            }
            "agent_end" if value.get("willRetry").and_then(Value::as_bool) != Some(true) => {
                self.fallback_agent_end_at = Some(now);
            }
            "agent_start"
            | "auto_retry_start"
            | "compaction_start"
            | "summarization_retry_scheduled" => {
                self.canonical_settled = false;
                self.fallback_agent_end_at = None;
            }
            "queue_update" if queue_has_pending_work(value) => {
                self.canonical_settled = false;
                self.fallback_agent_end_at = None;
            }
            _ => {}
        }
    }

    fn should_request_state(&self, now: Instant) -> bool {
        self.canonical_settled
            || self
                .fallback_agent_end_at
                .is_some_and(|seen| now.duration_since(seen) >= AGENT_END_SETTLE_FALLBACK)
    }

    fn saw_terminal_boundary(&self) -> bool {
        self.canonical_settled || self.fallback_agent_end_at.is_some()
    }
}

fn queue_has_pending_work(value: &Value) -> bool {
    ["steering", "followUp"].into_iter().any(|key| {
        value
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
    })
}

pub fn parse_pi_models(stderr: &str) -> Option<Vec<RuntimeModelOption>> {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if lines.len() <= 1 {
        return None;
    }
    let mut out = vec![default_model_option()];
    let mut seen = std::collections::HashSet::from(["default".to_string()]);
    for line in lines.iter().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let full_id = format!("{}/{}", parts[0], parts[1]);
        if seen.insert(full_id.clone()) {
            let context_window_tokens = parts
                .get(2)
                .and_then(|label| parse_context_window_label(label));
            out.push(RuntimeModelOption {
                id: full_id.clone(),
                label: full_id,
                context_window_tokens,
            });
        }
    }
    if out.len() > 1 {
        Some(out)
    } else {
        None
    }
}

pub fn map_pi_rpc_event(value: &Value, sink: &mut dyn FnMut(UnifiedAgentEvent)) -> PiRpcOutcome {
    let obj = match value.as_object() {
        Some(o) => o,
        None => return PiRpcOutcome::Continue,
    };
    let kind = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");

    const STATUS_EVENTS: &[&str] = &[
        "agent_start",
        "agent_end",
        "agent_settled",
        "turn_start",
        "turn_end",
        "message_start",
        "message_end",
        "tool_execution_update",
        "queue_update",
        "compaction_start",
        "compaction_end",
        "entry_appended",
        "session_info_changed",
        "thinking_level_changed",
        "auto_retry_start",
        "auto_retry_end",
        "summarization_retry_scheduled",
        "summarization_retry_attempt_start",
        "summarization_retry_finished",
        "bash_execution_update",
        "extension_error",
    ];
    if STATUS_EVENTS.contains(&kind) {
        sink(UnifiedAgentEvent::RuntimeStatus {
            kind: kind.to_string(),
            data: value.clone(),
        });
    }

    match kind {
        "agent_start" => {}
        "agent_end" => {}
        "agent_settled" => return PiRpcOutcome::AgentEnd,
        "turn_start" => {}
        "turn_end" => {
            if let Some(message) = obj.get("message").and_then(|v| v.as_object()) {
                if let Some(usage) = message.get("usage").and_then(|v| v.as_object()) {
                    let input = usage.get("input").and_then(|v| v.as_u64()).unwrap_or(0);
                    let output = usage.get("output").and_then(|v| v.as_u64()).unwrap_or(0);
                    if input > 0 || output > 0 {
                        sink(UnifiedAgentEvent::Usage {
                            usage: usage_from_numbers(input, output),
                        });
                    }
                }
                if message.get("stopReason").and_then(|v| v.as_str()) == Some("error") {
                    let message_text = message
                        .get("errorMessage")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Pi agent error");
                    sink(UnifiedAgentEvent::Error {
                        message: message_text.to_string(),
                    });
                }
            }
        }
        "message_update" => {
            if let Some(ev) = obj.get("assistantMessageEvent").and_then(|v| v.as_object()) {
                let ev_type = ev.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match ev_type {
                    "text_delta" => {
                        if let Some(delta) = ev.get("delta").and_then(|v| v.as_str()) {
                            sink(UnifiedAgentEvent::TextDelta {
                                delta: delta.to_string(),
                            });
                        }
                    }
                    "thinking_delta" => {
                        if let Some(delta) = ev.get("delta").and_then(|v| v.as_str()) {
                            sink(UnifiedAgentEvent::ThinkingDelta {
                                delta: delta.to_string(),
                            });
                        }
                    }
                    "error" => {
                        let message = ev
                            .get("reason")
                            .or_else(|| ev.get("delta"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("Agent error");
                        sink(UnifiedAgentEvent::Error {
                            message: message.to_string(),
                        });
                    }
                    _ => {}
                }
            }
        }
        "tool_execution_start" => {
            let id = obj
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = obj
                .get("toolName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let input = obj.get("args").cloned().unwrap_or(Value::Null);
            if !id.is_empty() && !name.is_empty() {
                sink(UnifiedAgentEvent::ToolUse { id, name, input });
            }
        }
        "tool_execution_end" => {
            let tool_use_id = obj
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let result = obj.get("result").cloned().unwrap_or(Value::Null);
            let content = pi_result_text(&result);
            let is_error = obj
                .get("isError")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !tool_use_id.is_empty() {
                sink(UnifiedAgentEvent::PiToolResult {
                    tool_use_id,
                    content,
                    is_error,
                    result,
                });
            }
        }
        "extension_error" => {
            let message = obj
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Extension error");
            sink(UnifiedAgentEvent::Error {
                message: message.to_string(),
            });
        }
        "auto_retry_end" if obj.get("success").and_then(|v| v.as_bool()) == Some(false) => {
            let message = obj
                .get("finalError")
                .and_then(|v| v.as_str())
                .unwrap_or("Auto-retry exhausted");
            sink(UnifiedAgentEvent::Error {
                message: message.to_string(),
            });
        }
        _ => {}
    }
    PiRpcOutcome::Continue
}

fn pi_result_text(result: &Value) -> String {
    match result.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn parse_extension_ui_request(raw: &Value) -> Option<PiExtensionUiRequest> {
    let options = raw
        .get("options")
        .or_else(|| raw.get("params").and_then(|params| params.get("options")))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|option| {
            option.as_str().map(str::to_string).or_else(|| {
                option
                    .as_object()
                    .and_then(|object| object.get("value").or_else(|| object.get("label")))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        })
        .collect();
    Some(PiExtensionUiRequest {
        id: raw.get("id")?.as_str()?.to_string(),
        method: raw.get("method")?.as_str()?.to_string(),
        title: raw
            .get("title")
            .or_else(|| raw.get("params").and_then(|params| params.get("title")))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        message: raw
            .get("message")
            .or_else(|| raw.get("params").and_then(|params| params.get("message")))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        placeholder: raw
            .get("placeholder")
            .or_else(|| {
                raw.get("params")
                    .and_then(|params| params.get("placeholder"))
            })
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        options,
    })
}

async fn reply_extension_ui<W>(
    stdin: &mut W,
    request: &PiExtensionUiRequest,
    decision: PiExtensionUiDecision,
) -> Result<(), String>
where
    W: AsyncWrite + Unpin,
{
    let result = match decision {
        PiExtensionUiDecision::Confirmed(confirmed) if request.method == "confirm" => {
            json!({ "confirmed": confirmed })
        }
        PiExtensionUiDecision::Value(value) if request.method != "confirm" => {
            json!({ "value": value })
        }
        _ => json!({ "cancelled": true }),
    };
    let mut payload = json!({ "type": "extension_ui_response", "id": request.id });
    if let Some(obj) = payload.as_object_mut() {
        if let Some(result_obj) = result.as_object() {
            for (k, v) in result_obj {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    let mut line = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())
}

pub async fn run_pi_rpc_session<H, F>(
    child: &mut Child,
    prompt: &str,
    _model: Option<&str>,
    mut sink: impl FnMut(UnifiedAgentEvent),
    mut handle_extension_ui: H,
    cancel_check: impl Fn() -> bool,
) -> Result<Option<PiRpcSessionState>, String>
where
    H: FnMut(PiExtensionUiRequest) -> F,
    F: Future<Output = PiExtensionUiDecision>,
{
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "stdin unavailable".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout unavailable".to_string())?;

    let prompt_line = {
        let payload = json!({ "id": 1, "type": "prompt", "message": prompt });
        let mut line = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        line.push('\n');
        line
    };
    stdin
        .write_all(prompt_line.as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    let result = drain_pi_rpc_output(
        stdout,
        &mut stdin,
        &mut sink,
        &mut handle_extension_ui,
        cancel_check,
    )
    .await;
    if matches!(&result, Err(err) if err == "cancelled") {
        let _ = child.start_kill();
    }
    result
}

async fn drain_pi_rpc_output<R, W, H, F>(
    stdout: R,
    stdin: &mut W,
    sink: &mut impl FnMut(UnifiedAgentEvent),
    handle_extension_ui: &mut H,
    cancel_check: impl Fn() -> bool,
) -> Result<Option<PiRpcSessionState>, String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    H: FnMut(PiExtensionUiRequest) -> F,
    F: Future<Output = PiExtensionUiDecision>,
{
    let mut reader = BufReader::new(stdout).lines();
    let mut state_requested = false;
    let mut terminal = PiTerminalTracker::default();

    loop {
        if cancel_check() {
            return Err("cancelled".to_string());
        }

        let line = match timeout(Duration::from_millis(200), reader.next_line()).await {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => {
                return Err(if terminal.saw_terminal_boundary() {
                    "pi_rpc_eof_before_session_state"
                } else {
                    "pi_rpc_eof_without_terminal_state"
                }
                .to_string())
            }
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => {
                if !state_requested && terminal.should_request_state(Instant::now()) {
                    state_requested = true;
                    request_pi_state(stdin).await?;
                }
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        terminal.observe(&value, Instant::now());
        #[cfg(debug_assertions)]
        eprintln!("[pi-rpc] event={event_type}");

        if value.get("type").and_then(|v| v.as_str()) == Some("extension_ui_request") {
            let Some(request) = parse_extension_ui_request(&value) else {
                continue;
            };
            if FIRE_AND_FORGET.contains(&request.method.as_str()) {
                continue;
            }
            let decision = handle_extension_ui(request.clone()).await;
            reply_extension_ui(stdin, &request, decision).await?;
            continue;
        }

        if value.get("type").and_then(|v| v.as_str()) == Some("response") {
            if value.get("success").and_then(|v| v.as_bool()) == Some(false) {
                let err = value
                    .get("error")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "prompt rejected".to_string());
                return Err(err);
            }
            if value.get("command").and_then(Value::as_str) == Some("get_state") {
                let data = value.get("data").and_then(Value::as_object);
                let session_state = data.and_then(|data| {
                    Some(PiRpcSessionState {
                        session_file: data.get("sessionFile")?.as_str()?.to_string(),
                        session_id: data.get("sessionId")?.as_str()?.to_string(),
                        session_name: data
                            .get("sessionName")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                });
                let Some(session_state) = session_state else {
                    return Err("pi_rpc_get_state_missing_session_reference".to_string());
                };
                let _ = stdin.shutdown().await;
                return Ok(Some(session_state));
            }
            continue;
        }

        if map_pi_rpc_event(&value, sink) == PiRpcOutcome::AgentEnd {
            if !state_requested {
                state_requested = true;
                request_pi_state(stdin).await?;
            }
        }
    }
}

async fn request_pi_state<W: AsyncWrite + Unpin>(stdin: &mut W) -> Result<(), String> {
    let mut request = json!({ "id": "beefex-get-state", "type": "get_state" }).to_string();
    request.push('\n');
    stdin
        .write_all(request.as_bytes())
        .await
        .map_err(|error| error.to_string())
}

/// Thin direct client for one long-lived Pi RPC process. One instance is owned by exactly one
/// Task actor; no renderer code or generic runtime adapter can access its pipes or broker.
pub struct PiRpcClient {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    stderr_task: tokio::task::JoinHandle<String>,
    broker: Option<PiProviderBroker>,
    next_id: u64,
    state: PiRpcSessionState,
}

impl PiRpcClient {
    pub(crate) async fn connect(
        mut child: Child,
        broker: Option<PiProviderBroker>,
    ) -> Result<Self, String> {
        let stderr_task = drain_stderr(&mut child);
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Pi stdin unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Pi stdout unavailable".to_string())?;
        let mut client = Self {
            child,
            stdin,
            lines: BufReader::new(stdout).lines(),
            stderr_task,
            broker,
            next_id: 1,
            state: PiRpcSessionState {
                session_file: String::new(),
                session_id: String::new(),
                session_name: None,
            },
        };
        let response = client.execute_command(PiRpcCommand::GetState).await?;
        client.state = parse_pi_session_state(&response)?;
        Ok(client)
    }

    pub fn session_state(&self) -> &PiRpcSessionState {
        &self.state
    }

    async fn write_command(&mut self, command: &PiRpcCommand) -> Result<String, String> {
        let id = format!("beefex-pi-{}", self.next_id);
        self.next_id += 1;
        let mut line = serde_json::to_string(&command.with_id(id.clone())?)
            .map_err(|error| error.to_string())?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|error| format!("Pi RPC write failed: {error}"))?;
        Ok(id)
    }

    pub async fn execute_command(&mut self, command: PiRpcCommand) -> Result<Value, String> {
        let target_id = self.write_command(&command).await?;
        loop {
            let line = match timeout(Duration::from_millis(250), self.lines.next_line()).await {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => {
                    return Err(self.exit_error("pi_rpc_eof_while_waiting_response").await)
                }
                Ok(Err(error)) => return Err(error.to_string()),
                Err(_) => {
                    if self
                        .child
                        .try_wait()
                        .map_err(|error| error.to_string())?
                        .is_some()
                    {
                        return Err(self
                            .exit_error("pi_rpc_child_exit_while_waiting_response")
                            .await);
                    }
                    continue;
                }
            };
            let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            if value.get("type").and_then(Value::as_str) == Some("extension_ui_request") {
                if let Some(request) = parse_extension_ui_request(&value) {
                    reply_extension_ui(&mut self.stdin, &request, PiExtensionUiDecision::Cancelled)
                        .await?;
                }
                continue;
            }
            if value.get("type").and_then(Value::as_str) != Some("response")
                || value.get("id").and_then(Value::as_str) != Some(target_id.as_str())
            {
                continue;
            }
            if value.get("success").and_then(Value::as_bool) == Some(false) {
                return Err(value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("Pi RPC command failed")
                    .to_string());
            }
            return Ok(value.get("data").cloned().unwrap_or(Value::Null));
        }
    }

    pub async fn run_turn(
        &mut self,
        prompt: &str,
        events: &mpsc::Sender<UnifiedAgentEvent>,
        extension_ui: &mpsc::Sender<PiExtensionUiExchange>,
        control: &mut mpsc::Receiver<SessionCommand>,
    ) -> Result<(), String> {
        let prompt_id = self
            .write_command(&PiRpcCommand::Prompt {
                message: prompt.to_string(),
            })
            .await?;
        let mut terminal = PiTerminalTracker::default();
        let mut state_request_id: Option<String> = None;
        let mut cancelled = false;
        let mut pending_rpc: std::collections::HashMap<
            String,
            oneshot::Sender<Result<Value, String>>,
        > = std::collections::HashMap::new();

        loop {
            while let Ok(command) = control.try_recv() {
                match command {
                    SessionCommand::Cancel => {
                        if !cancelled {
                            cancelled = true;
                            let _ = self.write_command(&PiRpcCommand::Abort).await;
                        }
                    }
                    SessionCommand::Close => return Err("closed".to_string()),
                    SessionCommand::RunTurn { done, .. } => {
                        let _ = done.send(Err("session busy".to_string()));
                    }
                    SessionCommand::Rpc { command, response } => {
                        if command.changes_session() {
                            let _ = response.send(Err(
                                "Pi session command is unavailable while a turn is running"
                                    .to_string(),
                            ));
                            continue;
                        }
                        if matches!(&command, PiRpcCommand::Abort) {
                            cancelled = true;
                        }
                        let id = self.write_command(&command).await?;
                        pending_rpc.insert(id, response);
                    }
                }
            }

            let line = match timeout(Duration::from_millis(200), self.lines.next_line()).await {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => {
                    return Err(self
                        .exit_error(if terminal.saw_terminal_boundary() {
                            "pi_rpc_eof_before_session_state"
                        } else {
                            "pi_rpc_eof_without_terminal_state"
                        })
                        .await)
                }
                Ok(Err(error)) => return Err(error.to_string()),
                Err(_) => {
                    if self
                        .child
                        .try_wait()
                        .map_err(|error| error.to_string())?
                        .is_some()
                    {
                        return Err(self
                            .exit_error("pi_rpc_child_exit_without_terminal_state")
                            .await);
                    }
                    if state_request_id.is_none() && terminal.should_request_state(Instant::now()) {
                        state_request_id = Some(self.write_command(&PiRpcCommand::GetState).await?);
                    }
                    continue;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            terminal.observe(&value, Instant::now());

            if value.get("type").and_then(Value::as_str) == Some("extension_ui_request") {
                let Some(request) = parse_extension_ui_request(&value) else {
                    continue;
                };
                if FIRE_AND_FORGET.contains(&request.method.as_str()) {
                    continue;
                }
                let (response_tx, response_rx) = oneshot::channel();
                extension_ui
                    .send(PiExtensionUiExchange {
                        request: request.clone(),
                        response: response_tx,
                    })
                    .await
                    .map_err(|_| "Pi extension UI host unavailable".to_string())?;
                let decision = response_rx
                    .await
                    .unwrap_or(PiExtensionUiDecision::Cancelled);
                reply_extension_ui(&mut self.stdin, &request, decision).await?;
                continue;
            }

            if value.get("type").and_then(Value::as_str) == Some("response") {
                let response_id = value.get("id").and_then(Value::as_str).unwrap_or_default();
                if let Some(response) = pending_rpc.remove(response_id) {
                    let result = if value.get("success").and_then(Value::as_bool) == Some(false) {
                        Err(value
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("Pi RPC command failed")
                            .to_string())
                    } else {
                        Ok(value.get("data").cloned().unwrap_or(Value::Null))
                    };
                    let _ = response.send(result);
                    continue;
                }
                if response_id == prompt_id
                    && value.get("success").and_then(Value::as_bool) == Some(false)
                {
                    return Err(value
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Pi prompt rejected")
                        .to_string());
                }
                if state_request_id.as_deref() == Some(response_id) {
                    if value.get("success").and_then(Value::as_bool) == Some(false) {
                        return Err("Pi state request failed".to_string());
                    }
                    self.state = parse_pi_session_state(value.get("data").unwrap_or(&Value::Null))?;
                    if self
                        .broker
                        .as_ref()
                        .is_some_and(PiProviderBroker::authorization_rejected)
                    {
                        return Err("managed_credential_rejected".to_string());
                    }
                    return if cancelled {
                        Err("cancelled".to_string())
                    } else {
                        Ok(())
                    };
                }
                continue;
            }

            let mut mapped = Vec::new();
            if map_pi_rpc_event(&value, &mut |event| mapped.push(event)) == PiRpcOutcome::AgentEnd
                && state_request_id.is_none()
            {
                state_request_id = Some(self.write_command(&PiRpcCommand::GetState).await?);
            }
            for event in mapped {
                events
                    .send(event)
                    .await
                    .map_err(|_| "Pi event receiver closed".to_string())?;
            }
        }
    }

    async fn exit_error(&mut self, reason: &str) -> String {
        let stderr = if self.stderr_task.is_finished() {
            (&mut self.stderr_task).await.unwrap_or_default()
        } else {
            String::new()
        };
        if stderr.trim().is_empty() {
            reason.to_string()
        } else {
            format!(
                "{reason}: {}",
                crate::external_agents::spawn::tail_chars(stderr.trim(), 2000)
            )
        }
    }

    pub async fn close(&mut self) {
        let _ = self.stdin.shutdown().await;
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

fn parse_pi_session_state(data: &Value) -> Result<PiRpcSessionState, String> {
    let session_file = data
        .get("sessionFile")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "pi_rpc_get_state_missing_session_reference".to_string())?;
    let session_id = data
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "pi_rpc_get_state_missing_session_reference".to_string())?;
    Ok(PiRpcSessionState {
        session_file: session_file.to_string(),
        session_id: session_id.to_string(),
        session_name: data
            .get("sessionName")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

pub fn spawn_pi_session_actor(mut client: PiRpcClient) -> mpsc::Sender<SessionCommand> {
    let (tx, mut rx) = mpsc::channel::<SessionCommand>(16);
    tokio::spawn(async move {
        while let Some(command) = rx.recv().await {
            match command {
                SessionCommand::RunTurn {
                    prompt,
                    events,
                    pi_extension_ui,
                    done,
                    ..
                } => {
                    let result = match pi_extension_ui {
                        Some(extension_ui) => {
                            client
                                .run_turn(&prompt, &events, &extension_ui, &mut rx)
                                .await
                        }
                        None => Err("Pi extension UI channel missing".to_string()),
                    };
                    let _ = done.send(result);
                }
                SessionCommand::Rpc { command, response } => {
                    let changes_session = command.changes_session();
                    let result = match client.execute_command(command).await {
                        Ok(data) if changes_session => {
                            match client.execute_command(PiRpcCommand::GetState).await {
                                Ok(state_data) => match parse_pi_session_state(&state_data) {
                                    Ok(state) => {
                                        client.state = state;
                                        Ok(serde_json::json!({
                                            "result": data,
                                            "sessionState": state_data,
                                        }))
                                    }
                                    Err(error) => Err(error),
                                },
                                Err(error) => Err(error),
                            }
                        }
                        other => other,
                    };
                    let _ = response.send(result);
                }
                SessionCommand::Cancel => {
                    let _ = client.execute_command(PiRpcCommand::Abort).await;
                }
                SessionCommand::Close => {
                    client.close().await;
                    return;
                }
            }
        }
        client.close().await;
    });
    tx
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::*;
    use tokio::io::{duplex, sink};

    #[test]
    fn typed_rpc_command_surface_matches_pinned_pi_protocol() {
        let commands = vec![
            PiRpcCommand::Prompt {
                message: "x".into(),
            },
            PiRpcCommand::Steer {
                message: "x".into(),
            },
            PiRpcCommand::FollowUp {
                message: "x".into(),
            },
            PiRpcCommand::Abort,
            PiRpcCommand::NewSession {
                parent_session: None,
            },
            PiRpcCommand::GetState,
            PiRpcCommand::SetModel {
                provider: "p".into(),
                model_id: "m".into(),
            },
            PiRpcCommand::CycleModel,
            PiRpcCommand::GetAvailableModels,
            PiRpcCommand::SetThinkingLevel {
                level: "high".into(),
            },
            PiRpcCommand::CycleThinkingLevel,
            PiRpcCommand::GetAvailableThinkingLevels,
            PiRpcCommand::SetSteeringMode { mode: "all".into() },
            PiRpcCommand::SetFollowUpMode {
                mode: "one-at-a-time".into(),
            },
            PiRpcCommand::Compact {
                custom_instructions: None,
            },
            PiRpcCommand::SetAutoCompaction { enabled: true },
            PiRpcCommand::SetAutoRetry { enabled: true },
            PiRpcCommand::AbortRetry,
            PiRpcCommand::Bash {
                command: "pwd".into(),
                exclude_from_context: Some(false),
            },
            PiRpcCommand::AbortBash,
            PiRpcCommand::GetSessionStats,
            PiRpcCommand::ExportHtml { output_path: None },
            PiRpcCommand::SwitchSession {
                session_path: "/tmp/session.jsonl".into(),
            },
            PiRpcCommand::Fork {
                entry_id: "entry".into(),
            },
            PiRpcCommand::Clone,
            PiRpcCommand::GetForkMessages,
            PiRpcCommand::GetEntries { since: None },
            PiRpcCommand::GetTree,
            PiRpcCommand::GetLastAssistantText,
            PiRpcCommand::SetSessionName {
                name: "Task".into(),
            },
            PiRpcCommand::GetMessages,
            PiRpcCommand::GetCommands,
        ];
        assert_eq!(commands.len(), 32);
        let names = commands
            .iter()
            .map(PiRpcCommand::command_name)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(names.len(), 32);
        let set_model = PiRpcCommand::SetModel {
            provider: "beefex-managed".into(),
            model_id: "gpt".into(),
        }
        .with_id("rpc-1".into())
        .unwrap();
        assert_eq!(set_model["type"], "set_model");
        assert_eq!(set_model["modelId"], "gpt");
        assert_eq!(set_model["id"], "rpc-1");
    }

    #[test]
    fn only_pi_session_branching_commands_rotate_the_persisted_reference() {
        assert!(PiRpcCommand::NewSession {
            parent_session: None
        }
        .changes_session());
        assert!(PiRpcCommand::SwitchSession {
            session_path: "session.jsonl".into()
        }
        .changes_session());
        assert!(PiRpcCommand::Fork {
            entry_id: "entry".into()
        }
        .changes_session());
        assert!(PiRpcCommand::Clone.changes_session());
        assert!(!PiRpcCommand::Compact {
            custom_instructions: None
        }
        .changes_session());
        assert!(!PiRpcCommand::GetState.changes_session());
    }

    #[tokio::test]
    #[ignore = "requires bundled Pi runtime fixture"]
    async fn live_task_actor_reuses_one_pi_process_for_two_turns() {
        use std::process::Stdio;

        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let bin = repo.join("node_modules/.bin/pi");
        assert!(bin.is_file(), "build Pi runtime before running this test");
        let root = std::env::temp_dir().join(format!("beefex-pi-actor-{}", uuid::Uuid::new_v4()));
        let sessions = root.join("sessions");
        let agent = root.join("agent");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::write(root.join("promotion.txt"), "before\n").unwrap();
        let policy = repo.join("src-tauri/resources/pi/beefex-policy-extension.ts");
        let provider = repo.join("src-tauri/tests/fixtures/pi-promotion-provider.mjs");
        let child = Command::new(bin)
            .args([
                "--mode",
                "rpc",
                "--model",
                "beefex-fixture/promotion",
                "--session-dir",
            ])
            .arg(&sessions)
            .arg("--extension")
            .arg(policy)
            .arg("--extension")
            .arg(provider)
            .current_dir(&root)
            .env("PI_CODING_AGENT_DIR", &agent)
            .env("PI_CODING_AGENT_SESSION_DIR", &sessions)
            .env("PI_SKIP_VERSION_CHECK", "1")
            .env("PI_TELEMETRY", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let process_id = child.id().unwrap();
        let client = PiRpcClient::connect(child, None).await.unwrap();
        let session_id = client.session_state().session_id.clone();
        let control = spawn_pi_session_actor(client);

        async fn turn(
            control: &mpsc::Sender<SessionCommand>,
            prompt: &str,
        ) -> Vec<UnifiedAgentEvent> {
            let (events_tx, mut events_rx) = mpsc::channel(64);
            let (ui_tx, mut ui_rx) = mpsc::channel(8);
            let (done_tx, done_rx) = oneshot::channel();
            control
                .send(SessionCommand::RunTurn {
                    prompt: prompt.into(),
                    model: None,
                    reasoning: None,
                    events: events_tx,
                    pi_extension_ui: Some(ui_tx),
                    done: done_tx,
                })
                .await
                .unwrap();
            let mut done_rx = done_rx;
            let mut events = Vec::new();
            loop {
                tokio::select! {
                    result = &mut done_rx => {
                        result.unwrap().unwrap();
                        while let Ok(event) = events_rx.try_recv() { events.push(event); }
                        return events;
                    }
                    Some(event) = events_rx.recv() => events.push(event),
                    Some(exchange) = ui_rx.recv() => {
                        let _ = exchange.response.send(PiExtensionUiDecision::Confirmed(true));
                    }
                }
            }
        }

        let first = turn(&control, "Run the deterministic promotion scenario.").await;
        let second = turn(&control, "Confirm the prior run is resumable.").await;
        assert!(first.iter().any(
            |event| matches!(event, UnifiedAgentEvent::ToolUse { name, .. } if name == "edit")
        ));
        assert!(second.iter().any(|event| matches!(event, UnifiedAgentEvent::TextDelta { delta } if delta.contains("Promotion fixture completed"))));
        let (response_tx, response_rx) = oneshot::channel();
        control
            .send(SessionCommand::Rpc {
                command: PiRpcCommand::GetState,
                response: response_tx,
            })
            .await
            .unwrap();
        let state = response_rx.await.unwrap().unwrap();
        assert_eq!(state["sessionId"], session_id);
        assert!(process_id > 0);
        let _ = control.send(SessionCommand::Close).await;
        assert_eq!(
            std::fs::read_to_string(root.join("promotion.txt")).unwrap(),
            "created-by-pi\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    #[ignore = "requires live pi CLI on PATH"]
    async fn live_detect_pi_commands() {
        let bin = std::process::Command::new("which")
            .arg("pi")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from)
            .expect("pi on PATH");
        let cmds = detect_pi_commands(&bin, &["--mode", "rpc"], &std::env::temp_dir(), 10)
            .await
            .expect("pi get_commands");
        eprintln!("pi commands: {}", cmds.len());
        for c in cmds.iter().take(8) {
            eprintln!("  {}", c.slash);
        }
        assert!(!cmds.is_empty());
    }

    #[test]
    fn parse_pi_models_from_tsv() {
        let stderr = "provider model context\nanthropic claude-sonnet-4-5 200K\nopenai gpt-5 128K";
        let models = parse_pi_models(stderr).unwrap();
        assert!(models.iter().any(|m| m.id == "anthropic/claude-sonnet-4-5"));
        assert!(models.iter().any(|m| m.id == "openai/gpt-5"));
        let claude = models
            .iter()
            .find(|m| m.id == "anthropic/claude-sonnet-4-5")
            .unwrap();
        assert_eq!(claude.context_window_tokens, Some(200_000));
    }

    #[test]
    fn map_pi_text_delta() {
        let raw = r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"hi"}}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let mut events = Vec::new();
        map_pi_rpc_event(&value, &mut |e| events.push(e));
        assert!(matches!(
            events.first(),
            Some(UnifiedAgentEvent::TextDelta { delta }) if delta == "hi"
        ));
    }

    #[test]
    fn map_pi_turn_end_error_emits_error_and_stop_does_not() {
        let error: Value = serde_json::from_str(
            r#"{"type":"turn_end","message":{"stopReason":"error","errorMessage":"server_is_overloaded"}}"#,
        )
        .unwrap();
        let mut events = Vec::new();
        map_pi_rpc_event(&error, &mut |event| events.push(event));
        assert!(events.iter().any(|event| matches!(
            event,
            UnifiedAgentEvent::Error { message } if message == "server_is_overloaded"
        )));

        let stop: Value =
            serde_json::from_str(r#"{"type":"turn_end","message":{"stopReason":"stop"}}"#).unwrap();
        events.clear();
        map_pi_rpc_event(&stop, &mut |event| events.push(event));
        assert!(!events
            .iter()
            .any(|event| matches!(event, UnifiedAgentEvent::Error { .. })));
    }

    #[test]
    fn map_pi_agent_settled() {
        let raw = r#"{"type":"agent_settled"}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let mut events = Vec::new();
        assert_eq!(
            map_pi_rpc_event(&value, &mut |event| events.push(event)),
            PiRpcOutcome::AgentEnd
        );
        assert!(matches!(
            events.first(),
            Some(UnifiedAgentEvent::RuntimeStatus { kind, .. }) if kind == "agent_settled"
        ));
    }

    #[test]
    fn map_pi_lifecycle_events_for_desktop_projection() {
        let value = json!({"type":"queue_update","steering":["redirect"],"followUp":[]});
        let mut events = Vec::new();
        assert_eq!(
            map_pi_rpc_event(&value, &mut |event| events.push(event)),
            PiRpcOutcome::Continue
        );
        assert!(matches!(
            events.first(),
            Some(UnifiedAgentEvent::RuntimeStatus { kind, data })
                if kind == "queue_update" && data == &value
        ));
    }

    #[tokio::test]
    async fn eof_after_agent_end_without_session_state_is_explicit_failure() {
        let (stdout_reader, mut stdout_writer) = duplex(1024);
        let writer = tokio::spawn(async move {
            stdout_writer
                .write_all(b"{\"type\":\"agent_end\"}\n")
                .await?;
            tokio::time::sleep(Duration::from_millis(20)).await;
            stdout_writer
                .write_all(b"{\"type\":\"response\",\"command\":\"prompt\",\"success\":true}\n")
                .await?;
            stdout_writer.shutdown().await
        });
        let mut stdin = sink();
        let mut events = Vec::new();

        let result = drain_pi_rpc_output(
            stdout_reader,
            &mut stdin,
            &mut |event| events.push(event),
            &mut |_| async { PiExtensionUiDecision::Cancelled },
            || false,
        )
        .await;

        assert_eq!(result, Err("pi_rpc_eof_before_session_state".to_string()));
        assert!(
            writer.await.unwrap().is_ok(),
            "trailing write must not hit EPIPE"
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events.first(),
            Some(UnifiedAgentEvent::RuntimeStatus { kind, .. }) if kind == "agent_end"
        ));
    }

    #[tokio::test]
    async fn cancellation_still_interrupts_post_agent_end_drain() {
        let (stdout_reader, mut stdout_writer) = duplex(1024);
        stdout_writer
            .write_all(b"{\"type\":\"agent_end\"}\n")
            .await
            .unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_signal = Arc::clone(&cancelled);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel_signal.store(true, Ordering::SeqCst);
        });
        let mut stdin = sink();

        let result = drain_pi_rpc_output(
            stdout_reader,
            &mut stdin,
            &mut |_| {},
            &mut |_| async { PiExtensionUiDecision::Cancelled },
            || cancelled.load(Ordering::SeqCst),
        )
        .await;

        assert_eq!(result, Err("cancelled".to_string()));
        drop(stdout_writer);
    }

    #[tokio::test]
    async fn agent_end_without_settled_uses_bounded_session_state_fallback() {
        let (stdout_reader, mut stdout_writer) = duplex(2048);
        let (mut stdin_reader, mut stdin_writer) = duplex(2048);
        let writer = tokio::spawn(async move {
            stdout_writer
                .write_all(b"{\"type\":\"agent_end\",\"willRetry\":false}\n")
                .await?;
            let mut request = String::new();
            BufReader::new(&mut stdin_reader)
                .read_line(&mut request)
                .await?;
            assert!(request.contains("beefex-get-state"));
            stdout_writer
                .write_all(
                    b"{\"type\":\"response\",\"command\":\"get_state\",\"success\":true,\"data\":{\"sessionFile\":\"/tmp/pi.jsonl\",\"sessionId\":\"pi-session-1\"}}\n",
                )
                .await?;
            stdout_writer.shutdown().await
        });

        let result = drain_pi_rpc_output(
            stdout_reader,
            &mut stdin_writer,
            &mut |_| {},
            &mut |_| async { PiExtensionUiDecision::Cancelled },
            || false,
        )
        .await
        .unwrap()
        .expect("session state");

        assert_eq!(result.session_id, "pi-session-1");
        assert!(writer.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn child_eof_without_terminal_state_is_explicit_failure() {
        let (stdout_reader, stdout_writer) = duplex(256);
        drop(stdout_writer);
        let mut stdin = sink();

        let result = drain_pi_rpc_output(
            stdout_reader,
            &mut stdin,
            &mut |_| {},
            &mut |_| async { PiExtensionUiDecision::Cancelled },
            || false,
        )
        .await;

        assert_eq!(result, Err("pi_rpc_eof_without_terminal_state".to_string()));
    }

    #[test]
    fn retry_compaction_and_pending_queue_revoke_premature_fallback() {
        let start = Instant::now();
        for reset_event in [
            json!({"type":"auto_retry_start"}),
            json!({"type":"compaction_start"}),
            json!({"type":"summarization_retry_scheduled"}),
            json!({"type":"queue_update","steering":["continue"],"followUp":[]}),
        ] {
            let mut tracker = PiTerminalTracker::default();
            tracker.observe(&json!({"type":"agent_end","willRetry":false}), start);
            tracker.observe(&reset_event, start + Duration::from_millis(10));
            assert!(!tracker.should_request_state(start + Duration::from_secs(2)));
            assert!(!tracker.saw_terminal_boundary());
        }
    }

    #[test]
    fn empty_queue_update_does_not_erase_completed_agent_end_candidate() {
        let start = Instant::now();
        let mut tracker = PiTerminalTracker::default();
        tracker.observe(&json!({"type":"agent_end","willRetry":false}), start);
        tracker.observe(
            &json!({"type":"queue_update","steering":[],"followUp":[]}),
            start + Duration::from_millis(10),
        );
        assert!(tracker.should_request_state(start + Duration::from_secs(2)));
    }

    #[test]
    fn parse_pi_models_real_aligned_table() {
        // Real `pi --list-models` output: header + 6 space-aligned columns.
        let out = "provider          model          context  max-out  thinking  images\n\
                   zmfooogreencloud  mimo-v2.5-pro  128K     8.2K     no        no\n\
                   zmfooogreencloud  minimax-m2.7   128K     8.2K     no        no";
        let models = parse_pi_models(out).unwrap();
        assert!(models
            .iter()
            .any(|m| m.id == "zmfooogreencloud/mimo-v2.5-pro"));
        assert!(models
            .iter()
            .any(|m| m.id == "zmfooogreencloud/minimax-m2.7"));
        // Generic provider models must NOT appear (those were the bogus fallback).
        assert!(!models.iter().any(|m| m.id.starts_with("anthropic/")));
    }

    #[test]
    fn parses_extension_confirmation_without_inventing_a_decision() {
        let raw = json!({
            "type": "extension_ui_request",
            "id": "approval-7",
            "method": "confirm",
            "title": "Allow bash?",
            "message": "npm test"
        });
        let request = parse_extension_ui_request(&raw).expect("request");
        assert_eq!(request.id, "approval-7");
        assert_eq!(request.method, "confirm");
        assert_eq!(request.title, "Allow bash?");
        assert_eq!(request.message, "npm test");
        assert!(request.placeholder.is_empty());
    }

    #[test]
    fn parses_bounded_client_setup_payload_from_pi_native_input() {
        let raw = json!({
            "type": "extension_ui_request",
            "id": "setup-1",
            "method": "input",
            "title": "__BEEFEX_MANAGED_CLIENTS_APPLY__",
            "placeholder": "{\"codexModel\":\"gpt-5.6-sol\"}"
        });
        let request = parse_extension_ui_request(&raw).expect("request");
        assert_eq!(request.method, "input");
        assert_eq!(request.title, "__BEEFEX_MANAGED_CLIENTS_APPLY__");
        assert_eq!(request.placeholder, "{\"codexModel\":\"gpt-5.6-sol\"}");
    }

    #[tokio::test]
    async fn extension_confirmation_uses_the_host_decision() {
        let (mut client, mut server) = duplex(1024);
        let request = PiExtensionUiRequest {
            id: "approval-8".to_string(),
            method: "confirm".to_string(),
            title: "Allow write?".to_string(),
            message: "src/demo.ts".to_string(),
            placeholder: String::new(),
            options: Vec::new(),
        };
        reply_extension_ui(
            &mut client,
            &request,
            PiExtensionUiDecision::Confirmed(false),
        )
        .await
        .unwrap();
        client.shutdown().await.unwrap();
        let mut line = String::new();
        BufReader::new(&mut server)
            .read_line(&mut line)
            .await
            .unwrap();
        let response: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(
            response.get("id").and_then(Value::as_str),
            Some("approval-8")
        );
        assert_eq!(
            response.get("confirmed").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[tokio::test]
    async fn settled_agent_captures_pi_owned_session_state() {
        let (stdout_reader, mut stdout_writer) = duplex(2048);
        let (mut stdin_reader, mut stdin_writer) = duplex(2048);
        let writer = tokio::spawn(async move {
            stdout_writer
                .write_all(b"{\"type\":\"agent_settled\"}\n")
                .await?;
            let mut request = String::new();
            BufReader::new(&mut stdin_reader)
                .read_line(&mut request)
                .await?;
            assert!(request.contains("beefex-get-state"));
            stdout_writer
                .write_all(
                    b"{\"type\":\"response\",\"command\":\"get_state\",\"success\":true,\"data\":{\"sessionFile\":\"/tmp/pi.jsonl\",\"sessionId\":\"pi-7\",\"sessionName\":\"Beefex\"}}\n",
                )
                .await?;
            stdout_writer.shutdown().await
        });

        let state = drain_pi_rpc_output(
            stdout_reader,
            &mut stdin_writer,
            &mut |_| {},
            &mut |_| async { PiExtensionUiDecision::Cancelled },
            || false,
        )
        .await
        .unwrap()
        .expect("session state");

        assert_eq!(state.session_id, "pi-7");
        assert_eq!(state.session_file, "/tmp/pi.jsonl");
        assert_eq!(state.session_name.as_deref(), Some("Beefex"));
        writer.await.unwrap().unwrap();
    }
}
