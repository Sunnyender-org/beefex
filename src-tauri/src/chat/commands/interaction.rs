use std::{collections::HashMap, time::Duration};

use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use tokio::time::{sleep, timeout};

use crate::chat::agent::execute::truncate_chars;
use crate::chat::{AgentPlanState, ChatMessageSegment, Conversation, ToolCallRecord};
use crate::mcp::types::ChatToolArtifact;
use crate::state::{AppState, ChatToolApprovalIdentity};

use super::catalog::strip_transcripts_for_frontend;
use crate::chat::storage::{load_conversation, save_conversation};

/// 取走外部入口排队给 Chat 前端发送的消息。
#[tauri::command]
pub(crate) fn chat_take_external_sends(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let requests = {
        let mut pending = state
            .pending_chat_external_sends
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *pending)
    };

    Ok(serde_json::json!({
        "success": true,
        "requests": requests,
    }))
}

#[tauri::command]
pub(crate) fn chat_set_agent_plan_mode(
    app: AppHandle,
    conversation_id: String,
    mode: String,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    let mode = crate::chat::plan::mode_from_str(&mode)?;
    conversation.agent_plan_state =
        crate::chat::plan::with_mode(&conversation.agent_plan_state, mode);
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;
    emit_chat_plan_state(&app, &conversation.id, &conversation.agent_plan_state);

    strip_transcripts_for_frontend(&mut conversation);
    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
        "planState": conversation.agent_plan_state,
    }))
}

#[tauri::command]
pub(crate) fn chat_execute_agent_plan(
    app: AppHandle,
    conversation_id: String,
    message_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    approve_agent_plan_for_execution(&mut conversation, message_id.as_deref())?;
    conversation.updated_at = chrono::Local::now().timestamp();
    save_conversation(&app, &conversation)?;
    emit_chat_plan_state(&app, &conversation.id, &conversation.agent_plan_state);

    strip_transcripts_for_frontend(&mut conversation);
    Ok(serde_json::json!({
        "success": true,
        "conversation": conversation,
        "planState": conversation.agent_plan_state,
    }))
}

pub(super) fn approve_agent_plan_for_execution(
    conversation: &mut Conversation,
    message_id: Option<&str>,
) -> Result<(), String> {
    let selected_plan =
        if let Some(message_id) = message_id.map(str::trim).filter(|id| !id.is_empty()) {
            Some({
                let message = conversation
                    .messages
                    .iter_mut()
                    .find(|message| message.id == message_id && message.role == "assistant")
                    .ok_or_else(|| "计划消息不存在".to_string())?;
                let plan_state = message
                    .agent_plan
                    .as_ref()
                    .ok_or_else(|| "该消息不是可执行计划".to_string())?;
                if crate::chat::plan::executable_plan_text(plan_state).is_none() {
                    return Err("该消息不是可执行计划".to_string());
                }
                let approved = crate::chat::plan::approve(plan_state);
                message.agent_plan = Some(approved.clone());
                approved
            })
        } else {
            None
        };
    conversation.agent_plan_state =
        selected_plan.unwrap_or_else(|| crate::chat::plan::approve(&conversation.agent_plan_state));
    Ok(())
}

/// 取消指定对话的当前 Chat 生成或工具执行。
#[tauri::command]
pub(crate) fn chat_cancel_stream(
    state: State<AppState>,
    conversation_id: String,
) -> Result<(), String> {
    state.cancel_chat_generation(&conversation_id);
    Ok(())
}

/// 响应敏感工具调用确认。
#[tauri::command]
pub(crate) fn chat_confirm_tool_call(
    state: State<AppState>,
    conversation_id: String,
    run_id: String,
    tool_call_id: String,
    approved: bool,
) -> Result<ChatToolApprovalResolution, String> {
    resolve_chat_tool_approval(
        state.inner(),
        ChatToolApprovalIdentity {
            conversation_id,
            run_id,
            tool_call_id,
        },
        approved,
    )
}

#[derive(Debug, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatToolApprovalResolution {
    pub decision: &'static str,
    pub reason_code: &'static str,
    pub approval_resolved: bool,
}

fn resolve_chat_tool_approval(
    state: &AppState,
    identity: ChatToolApprovalIdentity,
    approved: bool,
) -> Result<ChatToolApprovalResolution, String> {
    state.resolve_chat_tool_approval(&identity, approved)?;
    Ok(ChatToolApprovalResolution {
        decision: if approved { "allow" } else { "deny" },
        reason_code: "approval_matched_active_run",
        approval_resolved: true,
    })
}

/// 返回开发者「请求调试」缓冲快照（最新在前）。仅内存，未开启开关时通常为空。
#[tauri::command]
pub(crate) fn get_request_debug_records(
    state: State<AppState>,
) -> Vec<crate::chat::request_debug::RequestDebugRecord> {
    crate::chat::request_debug::snapshot(&state)
}

/// 清空开发者「请求调试」缓冲。
#[tauri::command]
pub(crate) fn clear_request_debug_records(state: State<AppState>) {
    crate::chat::request_debug::clear(&state);
}

/// 列出当前仍在运行的后台命令（chat agent 用 `run_command background:true` 起的）。
/// 只返回 Running 的——UI 仅在有后台任务时才显示指示器，终止/退出的不必展示。
#[tauri::command]
pub(crate) fn chat_list_background_commands(state: State<AppState>) -> Vec<serde_json::Value> {
    let map = state.background_commands_handle();
    let map = map.lock().unwrap_or_else(|e| e.into_inner());
    let mut jobs: Vec<&crate::native_tools::BackgroundCommand> = map
        .values()
        .filter(|j| {
            matches!(
                j.status,
                crate::native_tools::BackgroundCommandStatus::Running
            )
        })
        .collect();
    jobs.sort_by_key(|j| j.started_at);
    jobs.into_iter()
        .map(|j| {
            serde_json::json!({
                "jobId": j.job_id,
                "command": j.command,
                "cwd": j.cwd,
                "pid": j.pid,
                "elapsedSecs": j.started_at.elapsed().map(|d| d.as_secs()).unwrap_or(0),
            })
        })
        .collect()
}

/// 从 UI 终止一个后台命令。复用 agent 的 `kill_background`（整组杀 + 标记 Killed）。
#[tauri::command]
pub(crate) fn chat_kill_background_command(
    state: State<AppState>,
    job_id: String,
) -> Result<(), String> {
    crate::native_tools::kill_background(&state, &serde_json::json!({ "job_id": job_id }))
        .map(|_| ())
}

/// 响应会话级文件/命令工具授权请求(按 conversation_id)。
#[tauri::command]
pub(crate) fn chat_respond_session_consent(
    state: State<AppState>,
    conversation_id: String,
    granted: bool,
) -> Result<(), String> {
    let sender = state
        .pending_chat_session_consents
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&conversation_id);
    if let Some(sender) = sender {
        let _ = sender.send(granted);
    }
    Ok(())
}

/// 回答 ask_user 澄清卡片。
#[tauri::command]
pub(crate) fn chat_submit_user_choice(
    state: State<AppState>,
    tool_call_id: String,
    answers: HashMap<String, crate::chat::ask_user::AskUserAnswer>,
    skipped: bool,
) -> Result<(), String> {
    let response = {
        let pending = state
            .pending_chat_user_prompts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Some(pending) = pending.get(&tool_call_id) else {
            return Err("Clarification is no longer awaiting a response".to_string());
        };
        if skipped {
            crate::chat::ask_user::skipped_response()
        } else {
            crate::chat::ask_user::validate_response(
                &pending.prompt,
                crate::chat::ask_user::AskUserResponseResult {
                    phase: crate::chat::ask_user::ASK_USER_PHASE_ANSWERED.to_string(),
                    answers,
                },
            )?
        }
    };
    let pending = state
        .pending_chat_user_prompts
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&tool_call_id);
    let Some(pending) = pending else {
        return Err("Clarification is no longer awaiting a response".to_string());
    };
    let _ = pending.sender.send(response);
    Ok(())
}

/// 前端 Pyodide 执行完成后回传结果。
#[tauri::command]
pub(crate) fn chat_python_complete(
    state: State<AppState>,
    run_id: String,
    content: String,
    is_error: bool,
    artifacts: Option<Vec<ChatToolArtifact>>,
) -> Result<(), String> {
    let pending = state
        .pending_python_runs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&run_id);
    if let Some(pending) = pending {
        let _ = pending.sender.send(crate::mcp::types::PythonRunResult {
            content,
            is_error,
            artifacts: artifacts.unwrap_or_default(),
        });
    }
    Ok(())
}

pub(super) fn emit_chat_plan_state(
    app: &AppHandle,
    conversation_id: &str,
    plan_state: &AgentPlanState,
) {
    let _ = app.emit(
        "chat-plan",
        serde_json::json!({
            "conversationId": conversation_id,
            "planState": plan_state,
        }),
    );
}

pub(super) async fn request_session_consent(
    app: &AppHandle,
    state: &AppState,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    generation: u64,
) -> bool {
    // Already granted for this conversation — no prompt.
    if state.has_chat_consent(conversation_id) {
        return true;
    }
    // Serialize prompts so concurrent first-round tools (read/grep/find/ls run
    // in parallel) don't each insert a pending sender and clobber one another.
    // Whoever wins the lock prompts once; the rest re-check consent and reuse
    // the grant without a second dialog.
    let _prompt_guard = state.chat_consent_prompt_lock.lock().await;
    if state.has_chat_consent(conversation_id) {
        return true;
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut pending = state
            .pending_chat_session_consents
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Only one outstanding consent prompt per conversation.
        pending.insert(conversation_id.to_string(), tx);
    }
    let _ = app.emit(
        "chat-session-consent",
        serde_json::json!({
            "conversationId": conversation_id,
            "runId": run_id,
            "messageId": message_id,
        }),
    );
    let result = tokio::select! {
        result = timeout(Duration::from_secs(60), rx) => result,
        _ = wait_for_chat_cancel(state, conversation_id, generation) => {
            state
                .pending_chat_session_consents
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(conversation_id);
            return false;
        }
    };
    match result {
        Ok(Ok(true)) => {
            state.grant_chat_consent(conversation_id);
            true
        }
        _ => {
            state
                .pending_chat_session_consents
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(conversation_id);
            false
        }
    }
}

pub(crate) async fn request_tool_approval(
    app: &AppHandle,
    state: &AppState,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    generation: u64,
    record: &ToolCallRecord,
) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let identity = ChatToolApprovalIdentity {
        conversation_id: conversation_id.to_string(),
        run_id: run_id.to_string(),
        tool_call_id: record.id.clone(),
    };
    let arguments_preview = format_tool_approval_summary(record);
    if !state.register_chat_tool_approval(
        identity.clone(),
        generation,
        message_id,
        &arguments_preview,
        tx,
    ) {
        return false;
    }
    let _ = app.emit(
        "chat-tool-confirm",
        serde_json::json!({
            "conversationId": conversation_id,
            "runId": run_id,
            "messageId": message_id,
            "toolCallId": record.id,
            "name": record.name,
            "source": record.source,
            "serverId": record.server_id,
            "argumentsPreview": arguments_preview,
            "sensitivity": "sensitive",
        }),
    );
    let result = tokio::select! {
        result = timeout(Duration::from_secs(60), rx) => result,
        _ = wait_for_chat_cancel(state, conversation_id, generation) => {
            state.remove_chat_tool_approval(&identity);
            return false;
        }
    };
    match result {
        Ok(Ok(value)) => value,
        _ => {
            state.remove_chat_tool_approval(&identity);
            false
        }
    }
}

pub(super) async fn request_user_response(
    app: &AppHandle,
    state: &AppState,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    generation: u64,
    record: &ToolCallRecord,
    prompt: crate::chat::ask_user::AskUserPromptPayload,
) -> crate::chat::ask_user::AskUserResponseResult {
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut pending = state
            .pending_chat_user_prompts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pending.insert(
            record.id.clone(),
            crate::chat::ask_user::PendingAskUserPrompt {
                prompt: prompt.clone(),
                sender: tx,
            },
        );
    }

    let empty_answers = HashMap::new();
    let structured_content = crate::chat::ask_user::structured_content(
        &prompt,
        crate::chat::ask_user::ASK_USER_PHASE_AWAITING,
        &empty_answers,
    );
    let _ = app.emit(
        "chat-user-prompt",
        serde_json::json!({
            "conversationId": conversation_id,
            "runId": run_id,
            "messageId": message_id,
            "toolCallId": record.id,
            "id": record.id,
            "name": record.name,
            "source": record.source,
            "prompt": prompt,
            "structuredContent": structured_content,
        }),
    );

    let result = tokio::select! {
        result = timeout(Duration::from_secs(600), rx) => result,
        _ = wait_for_chat_cancel(state, conversation_id, generation) => {
            let mut pending = state
                .pending_chat_user_prompts
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            pending.remove(&record.id);
            return crate::chat::ask_user::cancelled_response();
        }
    };
    match result {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => {
            let mut pending = state
                .pending_chat_user_prompts
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            pending.remove(&record.id);
            crate::chat::ask_user::cancelled_response()
        }
        Err(_) => {
            let mut pending = state
                .pending_chat_user_prompts
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            pending.remove(&record.id);
            crate::chat::ask_user::timeout_response()
        }
    }
}

pub(super) async fn wait_for_chat_cancel(state: &AppState, conversation_id: &str, generation: u64) {
    while state.is_chat_generation_active(conversation_id, generation) {
        sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) fn emit_chat_tool_record(
    app: &AppHandle,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    record: &ToolCallRecord,
) {
    let _ = app.emit(
        "chat-tool",
        serde_json::json!({
            "conversationId": conversation_id,
            "runId": run_id,
            "messageId": message_id,
            "toolCallId": record.id,
            "id": record.id,
            "name": record.name,
            "source": record.source,
            "serverId": record.server_id,
            "status": record.status,
            "argumentsPreview": truncate_chars(&record.arguments, 800),
            "resultPreview": record.result_preview,
            "error": record.error,
            "startedAt": record.started_at,
            "completedAt": record.completed_at,
            "durationMs": record.duration_ms,
            "round": record.round,
            "sensitive": record.sensitive,
            "artifacts": record.artifacts,
            "traceId": record.trace_id,
            "spanId": record.span_id,
            "structuredContent": record.structured_content,
        }),
    );
}

pub(crate) fn emit_chat_stream_delta(
    app: &AppHandle,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    delta: &str,
    reasoning_delta: Option<&str>,
    segment: Option<&ChatMessageSegment>,
) {
    let _ = app.emit(
        "chat-stream",
        serde_json::json!({
            "conversationId": conversation_id,
            "runId": run_id,
            "messageId": message_id,
            "imageId": "",
            "kind": "answer",
            "delta": delta,
            "reasoningDelta": reasoning_delta,
            "segmentId": segment.map(|segment| segment.id.as_str()),
            "segmentKind": segment.map(|segment| &segment.kind),
            "phase": segment.map(|segment| &segment.phase),
            "order": segment.map(|segment| segment.order),
            "stepNumber": segment.and_then(|segment| segment.step_number),
            "round": segment.and_then(|segment| segment.round),
            "toolCallId": segment.and_then(|segment| segment.tool_call_id.as_deref()),
            "segment": segment,
        }),
    );
}

pub(crate) fn emit_chat_stream_done(
    app: &AppHandle,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    reason: &str,
    full: &str,
) {
    let _ = app.emit(
        "chat-stream",
        serde_json::json!({
            "conversationId": conversation_id,
            "runId": run_id,
            "messageId": message_id,
            "imageId": "",
            "kind": "answer",
            "delta": "",
            "done": true,
            "reason": reason,
            "full": full,
        }),
    );
}

pub(super) fn format_tool_approval_summary(record: &ToolCallRecord) -> String {
    let parsed = serde_json::from_str::<Value>(&record.arguments).ok();
    let mut lines = Vec::new();
    match record.name.as_str() {
        "bash" => {
            if let Some(command) = parsed
                .as_ref()
                .and_then(|value| value.get("command"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("Command: {command}"));
                let risks = advisory_shell_risks(command);
                if !risks.is_empty() {
                    lines.push(format!("Advisory risks: {}", risks.join(", ")));
                }
            }
            if let Some(cwd) = parsed
                .as_ref()
                .and_then(|value| value.get("cwd"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("Working directory: {cwd}"));
            }
        }
        "write" | "edit" | "read" => {
            if let Some(path) = parsed
                .as_ref()
                .and_then(|value| value.get("path"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("Path: {path}"));
            }
            if record.name == "edit" {
                // Current shape: edits: [{old_string, new_string}, ...]. Preview the
                // first edit's old_string; fall back to the legacy single-edit field.
                let first_old = parsed
                    .as_ref()
                    .and_then(|value| value.get("edits"))
                    .and_then(|value| value.as_array())
                    .and_then(|edits| edits.first())
                    .and_then(|edit| edit.get("old_string"))
                    .and_then(|value| value.as_str())
                    .or_else(|| {
                        parsed
                            .as_ref()
                            .and_then(|value| value.get("old_string").or_else(|| value.get("old")))
                            .and_then(|value| value.as_str())
                    })
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if let Some(old) = first_old {
                    lines.push(format!("Replace: {}", truncate_chars(old, 180)));
                }
            }
        }
        _ => {}
    }

    if lines.is_empty() {
        truncate_chars(&record.arguments, 800)
    } else if record.source == "pi_extension" {
        lines.join("\n")
    } else {
        let mut summary = lines.join("\n");
        summary.push_str("\n\nRaw arguments:\n");
        summary.push_str(&truncate_chars(&record.arguments, 800));
        summary
    }
}

fn advisory_shell_risks(command: &str) -> Vec<&'static str> {
    let lowered = command.to_ascii_lowercase();
    let network = [
        "curl ",
        "wget ",
        "git clone",
        "npm install",
        "pnpm add",
        "yarn add",
        "bun add",
    ]
    .iter()
    .any(|marker| lowered.contains(marker));
    let redirection = command.contains('>') || command.contains('<');
    let absolute_path = command.split_whitespace().any(|token| {
        let token = token.trim_matches(|character: char| {
            matches!(character, '\'' | '"' | '(' | ')' | ';' | ',')
        });
        token.starts_with('/')
            || token.starts_with("~/")
            || (token.len() > 2
                && token.as_bytes()[1] == b':'
                && matches!(token.as_bytes()[2], b'/' | b'\\'))
    });
    [
        network.then_some("network"),
        redirection.then_some("redirection"),
        absolute_path.then_some("absolute path"),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(test)]
mod scoped_approval_tests {
    use super::*;
    use crate::state::{ChatRunStatus, ChatToolApprovalIdentity};

    fn begin_pending(
        state: &AppState,
        conversation_id: &str,
        run_id: &str,
        tool_call_id: &str,
    ) -> tokio::sync::oneshot::Receiver<bool> {
        let generation = state.next_chat_generation(conversation_id);
        state.begin_chat_run(conversation_id, run_id, "message-1", generation);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        state.register_chat_tool_approval(
            ChatToolApprovalIdentity {
                conversation_id: conversation_id.to_string(),
                run_id: run_id.to_string(),
                tool_call_id: tool_call_id.to_string(),
            },
            generation,
            "message-1",
            "edit project/src/demo.ts",
            sender,
        );
        receiver
    }

    #[test]
    fn exact_active_approval_resolves_only_its_scoped_sender() {
        let state = crate::state::test_app_state();
        let mut receiver = begin_pending(&state, "task-a", "run-1", "tool-file-1");

        let resolution = resolve_chat_tool_approval(
            &state,
            ChatToolApprovalIdentity {
                conversation_id: "task-a".to_string(),
                run_id: "run-1".to_string(),
                tool_call_id: "tool-file-1".to_string(),
            },
            true,
        )
        .expect("matching active approval");

        assert_eq!(resolution.reason_code, "approval_matched_active_run");
        assert!(resolution.approval_resolved);
        assert_eq!(receiver.try_recv(), Ok(true));
    }

    #[test]
    fn mismatched_tool_identity_never_resolves_pending_sender() {
        let state = crate::state::test_app_state();
        let mut receiver = begin_pending(&state, "task-a", "run-1", "tool-file-1");

        let error = resolve_chat_tool_approval(
            &state,
            ChatToolApprovalIdentity {
                conversation_id: "task-a".to_string(),
                run_id: "run-1".to_string(),
                tool_call_id: "tool-other".to_string(),
            },
            true,
        )
        .expect_err("mismatched approval must fail closed");

        assert_eq!(error, "approval_identity_mismatch");
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn approval_cannot_cross_task_or_reused_tool_id_in_a_new_run() {
        let state = crate::state::test_app_state();
        let mut receiver = begin_pending(&state, "task-a", "run-1", "tool-file-1");

        for (conversation_id, run_id) in [("task-b", "run-1"), ("task-a", "run-2")] {
            assert_eq!(
                resolve_chat_tool_approval(
                    &state,
                    ChatToolApprovalIdentity {
                        conversation_id: conversation_id.to_string(),
                        run_id: run_id.to_string(),
                        tool_call_id: "tool-file-1".to_string(),
                    },
                    true,
                )
                .expect_err("cross-scope approval"),
                "approval_identity_mismatch"
            );
        }
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn terminal_runs_cannot_resolve_a_previous_approval() {
        for (status, expected) in [
            (ChatRunStatus::Interrupted, "run_not_active"),
            (ChatRunStatus::Cancelled, "run_cancelled"),
        ] {
            let state = crate::state::test_app_state();
            let _receiver = begin_pending(&state, "task-a", "run-1", "tool-file-1");
            state.set_chat_run_status("task-a", "run-1", status);

            let error = resolve_chat_tool_approval(
                &state,
                ChatToolApprovalIdentity {
                    conversation_id: "task-a".to_string(),
                    run_id: "run-1".to_string(),
                    tool_call_id: "tool-file-1".to_string(),
                },
                true,
            )
            .expect_err("terminal approval must fail closed");

            assert_eq!(error, expected);
        }
    }

    #[test]
    fn cancelling_task_denies_and_clears_pending_approval() {
        let state = crate::state::test_app_state();
        let mut receiver = begin_pending(&state, "task-a", "run-1", "tool-file-1");

        state.cancel_chat_generation("task-a");

        assert_eq!(receiver.try_recv(), Ok(false));
        assert_eq!(
            resolve_chat_tool_approval(
                &state,
                ChatToolApprovalIdentity {
                    conversation_id: "task-a".to_string(),
                    run_id: "run-1".to_string(),
                    tool_call_id: "tool-file-1".to_string(),
                },
                true,
            )
            .expect_err("cancelled approval"),
            "run_cancelled"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn approved_fixture_replays_all_five_cases_through_scoped_boundary() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/tool-approval-v1.json"
        ))
        .expect("approval fixture");
        let cases = fixture["cases"].as_array().expect("fixture cases");
        assert_eq!(cases.len(), 5);

        for case in cases {
            let root = std::env::temp_dir()
                .join(format!("beefex-approval-fixture-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(root.join("src")).expect("fixture project");
            std::fs::write(root.join("src/demo.ts"), "export const ok = false\n")
                .expect("fixture source");
            let workspace = crate::native_tools::NativeToolWorkspace::project(
                "proj_fixture".to_string(),
                "Fixture".to_string(),
                Some(root.to_string_lossy().into_owned()),
            );
            let pending = &case["input"]["pending"];
            let response = &case["input"]["response"];
            let state = crate::state::test_app_state();
            let conversation_id = pending["conversation_id"].as_str().expect("task id");
            let run_id = pending["run_id"].as_str().expect("run id");
            let tool_call_id = pending["tool_call_id"].as_str().expect("tool id");
            let mut receiver = begin_pending(&state, conversation_id, run_id, tool_call_id);
            match case["input"]["run_state"].as_str().expect("run state") {
                "interrupted" => {
                    state.set_chat_run_status(conversation_id, run_id, ChatRunStatus::Interrupted)
                }
                "cancelled" => {
                    state.set_chat_run_status(conversation_id, run_id, ChatRunStatus::Cancelled)
                }
                "awaiting_approval" => {}
                state => panic!("unexpected fixture run state: {state}"),
            }
            let identity = ChatToolApprovalIdentity {
                conversation_id: response["conversation_id"]
                    .as_str()
                    .expect("response task")
                    .to_string(),
                run_id: response["run_id"]
                    .as_str()
                    .expect("response run")
                    .to_string(),
                tool_call_id: response["tool_call_id"]
                    .as_str()
                    .expect("response tool")
                    .to_string(),
            };
            let expected = &case["expected"];
            let resolution = resolve_chat_tool_approval(
                &state,
                identity,
                response["approved"].as_bool().expect("approved"),
            );

            if expected["decision"] == "deny" {
                assert_eq!(
                    resolution.expect_err("fixture denial"),
                    expected["reason_code"].as_str().expect("denial code")
                );
                assert_ne!(receiver.try_recv(), Ok(true));
                assert_eq!(
                    std::fs::read_to_string(root.join("src/demo.ts")).expect("unchanged source"),
                    "export const ok = false\n"
                );
                let _ = std::fs::remove_dir_all(root);
                continue;
            }

            let resolution = resolution.expect("fixture approval");
            assert_eq!(resolution.reason_code, "approval_matched_active_run");
            assert_eq!(receiver.try_recv(), Ok(true));
            let tool = &case["input"]["tool"];
            if tool["kind"] == "edit_file" {
                let result = crate::native_tools::edit_file(
                    &workspace,
                    &serde_json::json!({
                        "path": "src/demo.ts",
                        "edits": [{
                            "old_string": "export const ok = false\n",
                            "new_string": "export const ok = true\n"
                        }]
                    }),
                )
                .expect("approved file edit");
                let receipt =
                    crate::chat::receipt::build_completion_receipt(&[successful_tool_record(
                        "edit",
                        serde_json::to_value(result).expect("result"),
                    )]);
                assert_eq!(receipt.changed_files[0].path, "src/demo.ts");
                assert!(receipt.changed_files[0].has_diff);
            } else {
                let result = crate::native_tools::run_command_with_receipt(
                    &workspace,
                    2_000,
                    &serde_json::json!({ "command": "printf safe" }),
                    None,
                )
                .await
                .expect("approved fixture command");
                let receipt =
                    crate::chat::receipt::build_completion_receipt(&[successful_tool_record(
                        "bash",
                        serde_json::json!({
                            "type": "command_execution",
                            "command": result.command,
                            "cwd": result.cwd,
                            "exit_status": result.exit_status,
                            "stdout": result.stdout,
                        }),
                    )]);
                assert_eq!(receipt.commands[0].command, "printf safe");
                assert_eq!(receipt.commands[0].cwd, ".");
                assert_eq!(receipt.commands[0].exit_status, Some(0));
                assert_eq!(receipt.commands[0].stdout, "safe");
            }
            let _ = std::fs::remove_dir_all(root);
        }
    }

    fn successful_tool_record(name: &str, structured_content: serde_json::Value) -> ToolCallRecord {
        ToolCallRecord {
            id: format!("tool-{name}"),
            name: name.to_string(),
            source: "native".to_string(),
            server_id: None,
            arguments: "{}".to_string(),
            status: crate::chat::ToolCallStatus::Success,
            result_preview: None,
            error: None,
            duration_ms: Some(1),
            started_at: Some(1),
            completed_at: Some(2),
            round: 1,
            sensitive: true,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: Some(structured_content),
        }
    }
}
