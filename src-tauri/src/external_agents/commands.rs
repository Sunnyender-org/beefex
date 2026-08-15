use std::path::Path;
use tauri::{AppHandle, Manager};

use crate::chat::storage::load_conversation;
use crate::external_agents::project_trust::{preview_project_trust, set_project_trust};
use crate::external_agents::session::live::SessionCommand;
use crate::external_agents::session::pi_rpc::PiRpcCommand;
use crate::external_agents::slash::list_external_cli_slash_commands;
use crate::state::AppState;

#[tauri::command]
pub async fn chat_list_external_cli_slash_commands(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    agent_id: String,
    conversation_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let (supports, commands, message) =
        list_external_cli_slash_commands(&app, &state, &agent_id, conversation_id.as_deref())
            .await?;
    Ok(serde_json::json!({
        "success": true,
        "supportsSlashCommands": supports,
        "commands": commands,
        "message": message,
    }))
}

#[tauri::command]
pub fn chat_pi_project_trust_preview(
    app: AppHandle,
    root_path: String,
) -> Result<serde_json::Value, String> {
    let preview = preview_project_trust(&app, Path::new(root_path.trim()))?;
    Ok(serde_json::json!({ "success": true, "trust": preview }))
}

#[tauri::command]
pub fn chat_pi_set_project_trust(
    app: AppHandle,
    root_path: String,
    trusted: Option<bool>,
) -> Result<serde_json::Value, String> {
    let preview = set_project_trust(&app, Path::new(root_path.trim()), trusted)?;
    Ok(serde_json::json!({ "success": true, "trust": preview }))
}

#[tauri::command]
pub async fn chat_pi_rpc_command(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    conversation_id: String,
    command: PiRpcCommand,
) -> Result<serde_json::Value, String> {
    use tokio::sync::oneshot;

    let conversation = load_conversation(&app, &conversation_id)?;
    if conversation.agent_runtime.external_agent_id.as_deref() != Some("pi") {
        return Err("当前 Task 不是 Pi Task".to_string());
    }
    let cwd = crate::external_agents::workspace::resolve_effective_cwd(
        &app,
        &conversation_id,
        conversation.project_id.as_deref(),
    )?;
    let command_name = command.command_name();
    match &command {
        PiRpcCommand::Prompt { .. } => return Err("Prompt 必须经过正常 Task 发送链路".to_string()),
        PiRpcCommand::SetModel { .. }
        | PiRpcCommand::CycleModel
        | PiRpcCommand::GetAvailableModels => return Err("模型由 BeefAPI 允许列表管理".to_string()),
        PiRpcCommand::Bash { .. } => {
            return Err("终端命令必须经过 Task 内的 scoped approval".to_string())
        }
        PiRpcCommand::ExportHtml {
            output_path: Some(_),
        } => return Err("导出位置必须由 Beefex 文件选择器决定".to_string()),
        PiRpcCommand::SwitchSession { session_path } => {
            let session_root = app
                .path()
                .app_data_dir()
                .map_err(|error| format!("app_data_dir unavailable: {error}"))?
                .join("pi-runtime")
                .join("sessions")
                .canonicalize()
                .map_err(|error| format!("Pi session directory unavailable: {error}"))?;
            let target = std::path::Path::new(session_path)
                .canonicalize()
                .map_err(|error| format!("Pi session unavailable: {error}"))?;
            if !target.starts_with(session_root) {
                return Err("Pi session 必须位于 Beefex session 目录".to_string());
            }
        }
        _ => {}
    }
    let control = state
        .pi_live_session_control(&conversation_id, &cwd.to_string_lossy())
        .ok_or_else(|| "Pi Task 尚未启动，请先发送一条任务消息".to_string())?;
    let (response_tx, response_rx) = oneshot::channel();
    control
        .send(SessionCommand::Rpc {
            command,
            response: response_tx,
        })
        .await
        .map_err(|_| "Pi Task actor is unavailable".to_string())?;
    let data = tokio::time::timeout(std::time::Duration::from_secs(120), response_rx)
        .await
        .map_err(|_| format!("Pi RPC {command_name} timed out"))?
        .map_err(|_| "Pi Task actor dropped the command".to_string())??;
    if let Some(session_state) = data.get("sessionState") {
        let session_file = session_state
            .get("sessionFile")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "Pi session command returned no session file".to_string())?;
        let mut stored = crate::external_agents::session::load_session(&app, &conversation_id)
            .ok_or_else(|| "Pi Task session receipt is missing".to_string())?;
        stored.session_id = session_file.to_string();
        crate::external_agents::session::save_session(&app, &stored)?;
        crate::external_agents::session::save_live_handle(
            &app,
            &conversation_id,
            &crate::external_agents::session::LiveSessionHandle {
                agent_id: "pi".to_string(),
                protocol: "pi_rpc".to_string(),
                native_id: session_file.to_string(),
                cwd: cwd.to_string_lossy().to_string(),
            },
        )?;
    }
    Ok(serde_json::json!({
        "success": true,
        "command": command_name,
        "data": data,
    }))
}
