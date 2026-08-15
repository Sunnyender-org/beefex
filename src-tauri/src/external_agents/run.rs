use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::Local;
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

use crate::chat::agent::AgentRunEntry;
use crate::chat::commands::{
    emit_chat_stream_delta, emit_chat_stream_done, emit_chat_tool_record, push_assistant_message,
};
use crate::chat::memory::l1_prompt_block;
use crate::chat::model::ModelUsage;
use crate::chat::storage::save_conversation;
use crate::chat::types::{
    ChatMessageSegment, ChatMessageSegmentKind, ChatMessageSegmentPhase, ToolCallRecord,
    ToolCallStatus,
};
use crate::chat::Conversation;
use crate::external_agents::detection::detect_single_agent;
use crate::external_agents::project_trust::require_project_trust;
use crate::external_agents::prompt::{
    compose_external_prompt, compose_external_prompt_passthrough, cwd_hint, is_cli_slash_input,
};
use crate::external_agents::registry::get_agent_def;
use crate::external_agents::session::acp::{run_acp_session, AcpMcpServer};
use crate::external_agents::session::codex_app_server::run_codex_app_server_session;
use crate::external_agents::session::pi_rpc::{
    spawn_pi_session_actor, PiExtensionUiDecision, PiExtensionUiExchange, PiExtensionUiRequest,
    PiRpcClient,
};
use crate::external_agents::session::{persist_delivered_session, resolve_agent_resume_context};
use crate::external_agents::skill_stage::{skill_cwd_alias_segment, stage_active_skill};
use crate::external_agents::slash::{self};
use crate::external_agents::spawn::{
    drain_stderr, read_stdout_lines, resolve_binary, spawn_agent, tail_chars, write_prompt_stdin,
};
use crate::external_agents::stream::create_stream_handler;
use crate::external_agents::types::{
    ExternalAgentSession, RuntimeBuildOptions, RuntimeContext, StreamFormat, UnifiedAgentEvent,
};
use crate::external_agents::workspace::{extra_allowed_dirs_for_agent, resolve_effective_cwd};
use crate::skills::read_skill_detail;
use crate::state::{AppState, ChatRunStatus};

#[derive(Debug, Clone)]
pub struct ExternalRunIdentity {
    pub run_id: String,
    pub generation: u64,
}

pub struct ExternalRunOutcome {
    pub run_id: String,
    pub status: ChatRunStatus,
    pub receipt: crate::chat::ChatCompletionReceipt,
}

fn host_skill_bridge_enabled(stream_format: StreamFormat) -> bool {
    stream_format != StreamFormat::PiRpc
}

fn managed_pi_runtime_env(
    runtime_root: &std::path::Path,
) -> Result<HashMap<String, String>, String> {
    let agent_dir = runtime_root.join("agent");
    let session_dir = runtime_root.join("sessions");
    let isolated_home = runtime_root.join("home");
    for directory in [&agent_dir, &session_dir, &isolated_home] {
        std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| error.to_string())?;
        }
    }

    let mut env = HashMap::new();
    for key in ["PATH", "TMPDIR", "LANG", "LC_ALL"] {
        if let Ok(value) = std::env::var(key) {
            env.insert(key.to_string(), value);
        }
    }
    // Pi intentionally discovers global skills from ~/.agents/skills in addition to
    // PI_CODING_AGENT_DIR. Give the managed child an app-owned HOME so an ambient
    // host Skill catalog cannot leak into the managed BeefAPI Task. Project-local
    // .pi/.agents resources remain Pi-native and continue to be governed by trust.
    env.insert(
        "HOME".to_string(),
        isolated_home.to_string_lossy().into_owned(),
    );
    env.insert(
        "PI_CODING_AGENT_DIR".to_string(),
        agent_dir.to_string_lossy().into_owned(),
    );
    env.insert(
        "PI_CODING_AGENT_SESSION_DIR".to_string(),
        session_dir.to_string_lossy().into_owned(),
    );
    env.insert("PI_SKIP_VERSION_CHECK".to_string(), "1".to_string());
    env.insert("PI_TELEMETRY".to_string(), "0".to_string());
    Ok(env)
}

pub async fn run_external_cli_slash_command(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation: &mut Conversation,
    slash_command: &str,
) -> Result<(), String> {
    if !is_cli_slash_input(slash_command) {
        return Err("外部 CLI slash 命令必须以 / 开头".to_string());
    }
    run_external_cli_reply(
        app,
        state,
        conversation,
        None,
        slash_command,
        None,
        AgentRunEntry::Send,
        None,
    )
    .await
    .map(|_| ())
}

pub async fn run_external_cli_reply(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation: &mut Conversation,
    title_from_first_user: Option<&str>,
    latest_user_message: &str,
    active_skill_id: Option<&str>,
    entry: AgentRunEntry,
    registered_run: Option<ExternalRunIdentity>,
) -> Result<ExternalRunOutcome, String> {
    let settings = state.settings_read().clone();
    let agent_id = conversation
        .agent_runtime
        .external_agent_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| "未选择外部 Agent".to_string())?;

    let def = get_agent_def(&agent_id).ok_or_else(|| format!("未知外部 Agent: {agent_id}"))?;

    let cwd = resolve_effective_cwd(app, &conversation.id, conversation.project_id.as_deref())?;
    if def.stream_format == StreamFormat::PiRpc && conversation.project_id.is_some() {
        let project_has_root = conversation
            .project_id
            .as_deref()
            .and_then(|id| crate::chat::storage::find_project_by_id(app, id).ok())
            .and_then(|project| project.root_path)
            .is_some_and(|root| !root.trim().is_empty());
        if project_has_root {
            require_project_trust(app, &cwd)?;
        }
    }
    let detected = detect_single_agent(def, &cwd).await;
    if !detected.available {
        return Err(format!(
            "{} 未安装或不可用，请确认 CLI 在 PATH 中。",
            def.name
        ));
    }

    let resolved_bin = resolve_binary(def)
        .await
        .ok_or_else(|| format!("无法定位 {} 可执行文件", def.bin))?;

    let pi_provider = if def.stream_format == StreamFormat::PiRpc {
        let provider = state
            .beefapi_account
            .resolve_managed_provider(Some(&conversation.model), |account_state| {
                crate::beefapi::account::emit_account_state(app, &account_state)
            })
            .await?;
        Some(provider)
    } else {
        None
    };
    let effective_external_model = pi_provider
        .as_ref()
        .and_then(|provider| provider.model())
        .map(|model| format!("beefex-managed/{model}"))
        .or_else(|| conversation.agent_runtime.external_model.clone());

    let is_slash = is_cli_slash_input(latest_user_message);
    // Pi owns tools, skills, extensions, and sessions. The host-side bridge is a Kivio
    // compatibility path and must never leak into a Pi-native run.
    let host_skill_bridge_enabled = host_skill_bridge_enabled(def.stream_format);

    let skill_detail = if is_slash || !host_skill_bridge_enabled {
        None
    } else if let Some(skill_id) = active_skill_id.filter(|s| !s.is_empty()) {
        read_skill_detail(app, &settings.chat_tools.skill_scan_paths, skill_id).ok()
    } else {
        None
    };

    let memory_body = if is_slash || !settings.chat_memory.enabled {
        String::new()
    } else {
        l1_prompt_block(app).unwrap_or(None).unwrap_or_default()
    };

    let mut daemon_instructions = String::new();
    if !is_slash {
        if !settings.chat.system_prompt.trim().is_empty() {
            daemon_instructions.push_str(settings.chat.system_prompt.trim());
            daemon_instructions.push_str("\n\n");
        }
        if !memory_body.trim().is_empty() {
            daemon_instructions.push_str("## Memory\n\n");
            daemon_instructions.push_str(memory_body.trim());
            daemon_instructions.push('\n');
        }
    }
    daemon_instructions.push_str(&cwd_hint(
        cwd.to_string_lossy().as_ref(),
        host_skill_bridge_enabled,
    ));

    let resume_ctx = resolve_agent_resume_context(
        app,
        &conversation.id,
        def.id,
        def.resumes_session_via_cli,
        &daemon_instructions,
        effective_external_model.as_deref(),
        is_slash,
    );

    let skill_dir = skill_detail.as_ref().and_then(|d| d.meta.path.clone());
    let skill_body = skill_detail.as_ref().map(|d| d.body.clone());
    let skill_folder = skill_dir.as_deref().map(skill_cwd_alias_segment);

    if !is_slash && host_skill_bridge_enabled {
        if let (Some(dir), Some(folder)) = (skill_dir.as_deref(), skill_folder.as_deref()) {
            let _ = stage_active_skill(&cwd, folder, std::path::Path::new(dir));
        }
    }

    let composed = if is_slash {
        compose_external_prompt_passthrough(latest_user_message)
    } else {
        compose_external_prompt(
            conversation,
            &daemon_instructions,
            skill_body.as_deref(),
            skill_dir.as_deref(),
            skill_folder.as_deref(),
            resume_ctx.skip_instructions,
            resume_ctx.is_resuming,
            latest_user_message,
        )
    };

    let extra_dirs = if host_skill_bridge_enabled {
        extra_allowed_dirs_for_agent(def, &settings.chat_tools.skill_scan_paths)
    } else {
        Vec::new()
    };
    let runtime_ctx = RuntimeContext {
        extra_allowed_dirs: extra_dirs,
        resume_session_id: resume_ctx.resume_session_id.clone(),
        new_session_id: resume_ctx.new_session_id.clone(),
        include_partial_messages: true,
    };

    let build_options = RuntimeBuildOptions {
        model: effective_external_model.clone(),
        reasoning: conversation.agent_runtime.external_reasoning.clone(),
        sandbox: conversation.agent_runtime.external_sandbox.clone(),
    };

    if let Some(max_bytes) = def.max_prompt_arg_bytes {
        if composed.full_prompt.len() > max_bytes {
            return Err(format!(
                "Prompt 过长（{} 字节），超过 {} 的上限（{} 字节）。请缩短消息或改用 stdin 模式的 Agent。",
                composed.full_prompt.len(),
                def.name,
                max_bytes
            ));
        }
    }

    let prompt_for_args = if def.prompt_via_stdin {
        None
    } else {
        Some(composed.full_prompt.as_str())
    };
    let mut args = (def.build_args)(&runtime_ctx, &build_options, prompt_for_args);
    if def.stream_format == StreamFormat::PiRpc {
        let policy_extension = resolve_pi_policy_extension(app)?;
        args.push("--extension".to_string());
        args.push(policy_extension.to_string_lossy().to_string());
        let provider_extension = resolve_pi_managed_provider_extension(app)?;
        args.push("--extension".to_string());
        args.push(provider_extension.to_string_lossy().to_string());
    }

    let extra_env = if def.stream_format == StreamFormat::PiRpc {
        let runtime_root = app
            .path()
            .app_data_dir()
            .map_err(|error| format!("app_data_dir unavailable: {error}"))?
            .join("pi-runtime");
        managed_pi_runtime_env(&runtime_root)?
    } else {
        HashMap::new()
    };

    let owns_run = registered_run.is_none();
    let run_identity = registered_run.unwrap_or_else(|| ExternalRunIdentity {
        generation: state.next_chat_generation(&conversation.id),
        run_id: format!("ext-run-{}", Uuid::new_v4()),
    });
    let run_generation = run_identity.generation;
    let run_id = run_identity.run_id;
    let assistant_message_id = format!("msg_{}", Uuid::new_v4());
    if owns_run {
        state.begin_chat_run(
            &conversation.id,
            &run_id,
            &assistant_message_id,
            run_generation,
        );
    }

    // Rich protocols, including Pi RPC, keep one process actor per active Task.
    let persistent = matches!(
        def.stream_format,
        StreamFormat::CodexAppServer | StreamFormat::AcpJsonRpc | StreamFormat::PiRpc
    );
    let mut spawned_opt = if persistent {
        None
    } else {
        Some(spawn_agent(def, &resolved_bin, &args, &cwd, &extra_env).await?)
    };
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut raw_output = String::new();
    let mut tool_calls: Vec<ToolCallRecord> = Vec::new();
    let mut tool_map: HashMap<String, usize> = HashMap::new();
    let mut usage: Option<ModelUsage> = None;
    let mut stream_outcome = "completed".to_string();
    let mut stream_error: Option<String> = None;
    let mut segment_order = 0u32;
    let mut segments: Vec<ChatMessageSegment> = Vec::new();
    let mut segment_tracker = StreamSegmentTracker::default();
    let conversation_id = conversation.id.clone();
    let started_at = Instant::now();
    let slash_cache_key = slash::cache_key(&agent_id, &cwd.to_string_lossy());

    let mut emit_event = |event: UnifiedAgentEvent| {
        if let Some(commands) = slash::slash_commands_from_event(&event) {
            state.set_cached_external_slash_commands(slash_cache_key.clone(), commands);
        }
        if def.stream_format == StreamFormat::PiRpc {
            if let UnifiedAgentEvent::RuntimeStatus { kind, .. } = &event {
                let transition = match kind.as_str() {
                    "auto_retry_start" => Some("retry_started"),
                    "compaction_start" => Some("compaction_started"),
                    "summarization_retry_scheduled" => Some("retry_scheduled"),
                    _ => None,
                };
                if let Some(transition) = transition {
                    let mut diagnostic_roots = crate::diagnostics::default_private_roots();
                    diagnostic_roots.push(cwd.clone());
                    crate::diagnostics::record_app_event(
                        app,
                        crate::diagnostics::DiagnosticKind::PiChildLifecycle,
                        crate::diagnostics::DiagnosticLevel::Info,
                        transition,
                        None,
                        Some("pi_actor_progress"),
                        &diagnostic_roots,
                    );
                }
            }
        }
        apply_unified_event(
            app,
            &conversation_id,
            &run_id,
            &assistant_message_id,
            &mut content,
            &mut reasoning,
            &mut raw_output,
            &mut tool_calls,
            &mut tool_map,
            &mut usage,
            &mut stream_error,
            &mut segments,
            &mut segment_order,
            &mut segment_tracker,
            &cwd,
            event,
        );
    };

    let cancel_check = || !state.is_chat_generation_active(&conversation_id, run_generation);
    let delivered_instructions = if composed.instructions_block.is_empty() {
        daemon_instructions.as_str()
    } else {
        composed.instructions_block.as_str()
    };

    // Drain stderr concurrently with the stdout read below: keeps a full stderr pipe from
    // blocking the child, and captures failure text a silent (non-JSON, empty-stdout) run would
    // otherwise lose. Persistent protocols manage their own process, so there's no child here.
    let stderr_task = spawned_opt
        .as_mut()
        .map(|spawned| drain_stderr(&mut spawned.child));

    if def.stream_format == StreamFormat::PiRpc && resume_ctx.is_resuming {
        let mut diagnostic_roots = crate::diagnostics::default_private_roots();
        diagnostic_roots.push(cwd.clone());
        crate::diagnostics::record_app_event(
            app,
            crate::diagnostics::DiagnosticKind::TaskRecovery,
            crate::diagnostics::DiagnosticLevel::Info,
            "resume_requested",
            None,
            Some("pi_session_resume_requested"),
            &diagnostic_roots,
        );
    }

    let read_result = if def.stream_format == StreamFormat::PiRpc {
        run_persistent_pi_turn(
            app,
            state,
            &conversation_id,
            &resolved_bin,
            &args,
            &cwd,
            &extra_env,
            pi_provider.ok_or_else(|| "pi_provider_missing".to_string())?,
            &composed.full_prompt,
            latest_user_message,
            delivered_instructions,
            resume_ctx.delivered_model.clone(),
            &run_id,
            &assistant_message_id,
            run_generation,
            &mut emit_event,
            &cancel_check,
        )
        .await
    } else if persistent {
        let persistent_mcp: Vec<AcpMcpServer> = vec![];
        run_persistent_turn(
            app,
            state,
            &conversation_id,
            &agent_id,
            def.stream_format,
            &resolved_bin,
            &args,
            &cwd,
            effective_external_model.clone(),
            conversation.agent_runtime.external_reasoning.clone(),
            conversation.agent_runtime.external_sandbox.clone(),
            persistent_mcp,
            &composed.full_prompt,
            latest_user_message,
            &mut emit_event,
            &cancel_check,
        )
        .await
    } else {
        let spawned = spawned_opt
            .as_mut()
            .expect("non-persistent path spawns a child");
        match def.stream_format {
            StreamFormat::PiRpc => unreachable!("Pi RPC uses the persistent Task actor"),
            StreamFormat::CodexAppServer => {
                let model = conversation.agent_runtime.external_model.as_deref();
                let reasoning = conversation.agent_runtime.external_reasoning.as_deref();
                run_codex_app_server_session(
                    &mut spawned.child,
                    &composed.full_prompt,
                    model,
                    reasoning,
                    &cwd,
                    |event| emit_event(event),
                    cancel_check,
                )
                .await
            }
            StreamFormat::AcpJsonRpc => {
                let model = conversation.agent_runtime.external_model.as_deref();
                let mcp_servers: Vec<AcpMcpServer> = vec![];
                run_acp_session(
                    &mut spawned.child,
                    &composed.full_prompt,
                    &cwd,
                    model,
                    &mcp_servers,
                    |event| emit_event(event),
                    cancel_check,
                )
                .await
            }
            _ => {
                if def.prompt_via_stdin {
                    write_prompt_stdin(&mut spawned.child, def, &composed.full_prompt).await?;
                }
                let mut handler = create_stream_handler(def.stream_format, def.json_event_parser);
                read_stdout_lines(
                    &mut spawned.child,
                    |line| {
                        handler.handle_line(line, &mut |event| emit_event(event));
                        Ok(())
                    },
                    cancel_check,
                )
                .await
            }
        }
    };

    // Non-persistent path waits on (and drops/kills) the per-turn child. Persistent sessions
    // keep their process alive in the registry, so there is nothing to wait on here.
    let mut child_exit_forced = false;
    let exit_code: Option<i32> = match spawned_opt {
        Some(mut spawned) => {
            let status = if def.stream_format == StreamFormat::PiRpc {
                match tokio::time::timeout(Duration::from_secs(5), spawned.child.wait()).await {
                    Ok(result) => result.map_err(|error| error.to_string())?,
                    Err(_) => {
                        child_exit_forced = true;
                        let _ = spawned.child.start_kill();
                        spawned
                            .child
                            .wait()
                            .await
                            .map_err(|error| error.to_string())?
                    }
                }
            } else {
                spawned
                    .child
                    .wait()
                    .await
                    .map_err(|error| error.to_string())?
            };
            #[cfg(debug_assertions)]
            eprintln!(
                "[external-agent] agent={} child_exit={:?} forced={}",
                def.id,
                status.code(),
                child_exit_forced
            );
            status.code()
        }
        None => None,
    };
    let stderr_output = match stderr_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };
    if matches!(&read_result, Err(error) if error == "managed_credential_rejected") {
        let account_state = state
            .beefapi_account
            .handle_inference_credential_rejected(|account_state| {
                crate::beefapi::account::emit_account_state(app, &account_state)
            })
            .await;
        stream_error = Some(
            account_state
                .reason
                .unwrap_or_else(|| "reauthorization_required".to_string()),
        );
    }
    if let Err(err) = &read_result {
        stream_outcome = if err == "cancelled" {
            "cancelled"
        } else {
            "error"
        }
        .to_string();
        if err != "cancelled" && content.trim().is_empty() && raw_output.trim().is_empty() {
            raw_output = format!("{} 读取输出失败：{}", def.name, err);
        }
    } else if stream_error.is_some() {
        stream_outcome = "error".to_string();
    } else if child_exit_forced {
        stream_outcome = "error".to_string();
        if !content.trim().is_empty() {
            content.push_str("\n\n");
        }
        content.push_str("Pi 运行失败：pi_rpc_child_exit_timeout_without_session_state");
    } else if exit_code.map(|code| code != 0).unwrap_or(false) {
        if content.trim().is_empty() {
            stream_outcome = "error".to_string();
        }
    }

    // Fill empty content from the richest available fallback: captured raw stdout lines first,
    // then stderr (as an explicit failure), then the slash / no-output placeholders.
    if content.trim().is_empty() {
        if !raw_output.trim().is_empty() {
            content = raw_output.trim().to_string();
        } else if !stderr_output.trim().is_empty() {
            stream_outcome = "error".to_string();
            content = format!(
                "{} 执行失败：\n\n{}",
                def.name,
                truncate_for_preview(stderr_output.trim(), 4000)
            );
        } else if stream_outcome == "completed" {
            if is_slash {
                content = format!("{} 命令已执行", def.name);
            } else {
                stream_outcome = "error".to_string();
                content = format!(
                    "{} 未产生输出（exit={:?}，耗时 {}ms）",
                    def.name,
                    exit_code,
                    started_at.elapsed().as_millis()
                );
            }
        }
    }

    // A nonzero exit with stderr is a failure even if the CLI also produced some stdout — append
    // the stderr (unless it's already the content) so the error is visible, not swallowed.
    if exit_code.map(|code| code != 0).unwrap_or(false) && !stderr_output.trim().is_empty() {
        stream_outcome = "error".to_string();
        if !content.contains(stderr_output.trim()) {
            if !content.trim().is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(&format!(
                "{} stderr：\n\n{}",
                def.name,
                truncate_for_preview(stderr_output.trim(), 4000)
            ));
        }
    }

    emit_chat_stream_done(
        app,
        &conversation_id,
        &run_id,
        &assistant_message_id,
        &stream_outcome,
        &content,
    );

    if def.stream_format != StreamFormat::PiRpc {
        persist_delivered_session(
            app,
            &conversation_id,
            def.id,
            &resume_ctx,
            delivered_instructions,
            is_slash,
        )?;
    }

    let receipt = crate::chat::receipt::build_completion_receipt(&tool_calls);
    let terminal_status = match stream_outcome.as_str() {
        "completed" | "recovered" => ChatRunStatus::Completed,
        "cancelled" => ChatRunStatus::Cancelled,
        _ => ChatRunStatus::Failed,
    };
    promote_trailing_external_text_to_synthesis(&mut segments);
    push_assistant_message(
        app,
        state,
        &settings,
        conversation,
        assistant_message_id,
        content,
        if reasoning.is_empty() {
            None
        } else {
            Some(reasoning)
        },
        vec![],
        tool_calls,
        vec![],
        segments,
        active_skill_id.filter(|_| host_skill_bridge_enabled),
        title_from_first_user,
        Some(match entry {
            AgentRunEntry::Send => "send",
            AgentRunEntry::Regenerate => "regenerate",
        }),
        Some(&stream_outcome),
        usage,
        None,
        None,
    )
    .await?;

    save_conversation(app, conversation)?;
    state.set_chat_run_status(&conversation_id, &run_id, terminal_status);
    if def.stream_format == StreamFormat::PiRpc {
        let mut diagnostic_roots = crate::diagnostics::default_private_roots();
        diagnostic_roots.push(cwd.clone());
        let (level, transition) = match terminal_status {
            ChatRunStatus::Completed => (crate::diagnostics::DiagnosticLevel::Info, "completed"),
            ChatRunStatus::Cancelled => (crate::diagnostics::DiagnosticLevel::Info, "cancelled"),
            ChatRunStatus::Failed | ChatRunStatus::Interrupted => {
                (crate::diagnostics::DiagnosticLevel::Error, "failed")
            }
            _ => (crate::diagnostics::DiagnosticLevel::Warn, "terminal"),
        };
        crate::diagnostics::record_app_event(
            app,
            crate::diagnostics::DiagnosticKind::RunTerminal,
            level,
            transition,
            None,
            Some(&stream_outcome),
            &diagnostic_roots,
        );
        if resume_ctx.is_resuming {
            crate::diagnostics::record_app_event(
                app,
                crate::diagnostics::DiagnosticKind::TaskRecovery,
                level,
                if terminal_status == ChatRunStatus::Completed {
                    "recovered"
                } else {
                    "recovery_failed"
                },
                None,
                Some(if terminal_status == ChatRunStatus::Completed {
                    "pi_session_recovered"
                } else {
                    "pi_session_recovery_failed"
                }),
                &diagnostic_roots,
            );
        }
    }
    Ok(ExternalRunOutcome {
        run_id,
        status: terminal_status,
        receipt,
    })
}

async fn handle_pi_extension_ui(
    app: &AppHandle,
    state: &AppState,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    generation: u64,
    request: PiExtensionUiRequest,
) -> PiExtensionUiDecision {
    // Pi's RPC dialog id is the stable approval identity. Never guess a selection or
    // auto-confirm: unsupported dialogs fail closed, while confirmations reuse the one
    // AppState approval store and the existing renderer command.
    if request.method != "confirm" {
        return PiExtensionUiDecision::Cancelled;
    }
    let (tool_name, tool_arguments) = pi_extension_tool_record_fields(&request.message);
    let structured_content = serde_json::json!({
        "type": "pi_extension_confirmation",
        "requestId": request.id,
        "title": request.title,
        "message": request.message,
        "options": request.options,
    });
    let record = ToolCallRecord {
        id: request.id,
        name: tool_name,
        source: "pi_extension".to_string(),
        server_id: None,
        arguments: tool_arguments.to_string(),
        status: ToolCallStatus::Pending,
        result_preview: None,
        error: None,
        duration_ms: None,
        started_at: Some(Local::now().timestamp()),
        completed_at: None,
        round: 1,
        sensitive: true,
        artifacts: Vec::new(),
        trace_id: None,
        span_id: None,
        structured_content: Some(structured_content),
    };
    let approved = crate::chat::commands::interaction::request_tool_approval(
        app,
        state,
        conversation_id,
        run_id,
        message_id,
        generation,
        &record,
    )
    .await;
    PiExtensionUiDecision::Confirmed(approved)
}

fn pi_extension_tool_record_fields(message: &str) -> (String, serde_json::Value) {
    let tool_payload = serde_json::from_str::<serde_json::Value>(message).ok();
    let tool_name = tool_payload
        .as_ref()
        .and_then(|value| value.get("toolName"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("pi_extension_confirm")
        .to_string();
    let tool_arguments = tool_payload
        .as_ref()
        .and_then(|value| value.get("input"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    (tool_name, tool_arguments)
}

fn resolve_pi_policy_extension(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let bundled = app
        .path()
        .resource_dir()
        .ok()
        .map(|root| root.join("pi").join("beefex-policy-extension.ts"));
    let development = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("pi")
        .join("beefex-policy-extension.ts");
    bundled
        .filter(|path| path.is_file())
        .or_else(|| development.is_file().then_some(development))
        .ok_or_else(|| {
            "Pi policy extension is missing; refusing to start an unscoped Pi session".to_string()
        })
}

fn resolve_pi_managed_provider_extension(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let file_name = "beefex-managed-provider-extension.ts";
    let bundled = app
        .path()
        .resource_dir()
        .ok()
        .map(|root| root.join("pi").join(file_name));
    let development = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("pi")
        .join(file_name);
    bundled
        .filter(|path| path.is_file())
        .or_else(|| development.is_file().then_some(development))
        .ok_or_else(|| {
            "Pi managed provider extension is missing; refusing to expose managed credentials"
                .to_string()
        })
}

#[derive(Default)]
struct StreamSegmentTracker {
    active_text_idx: Option<usize>,
    active_reasoning_idx: Option<usize>,
}

impl StreamSegmentTracker {
    fn reset_text(&mut self) {
        self.active_text_idx = None;
    }

    fn reset_reasoning(&mut self) {
        self.active_reasoning_idx = None;
    }

    fn append(
        &mut self,
        kind: ChatMessageSegmentKind,
        segments: &mut Vec<ChatMessageSegment>,
        segment_order: &mut u32,
        tool_calls_len: usize,
        delta: &str,
    ) -> ChatMessageSegment {
        let phase = text_phase_for_tool_count(tool_calls_len);
        let active = match kind {
            ChatMessageSegmentKind::Reasoning => &mut self.active_reasoning_idx,
            _ => &mut self.active_text_idx,
        };
        if let Some(idx) = *active {
            if let Some(segment) = segments.get_mut(idx) {
                if segment.kind == kind && segment.phase == phase {
                    let merged = format!("{}{}", segment.text.as_deref().unwrap_or(""), delta);
                    segment.text = Some(merged);
                    return segment.clone();
                }
            }
        }

        *segment_order += 1;
        let segment = ChatMessageSegment {
            id: format!("seg_{}", Uuid::new_v4()),
            kind,
            phase,
            order: *segment_order,
            step_number: None,
            round: if tool_calls_len == 0 { None } else { Some(1) },
            text: Some(delta.to_string()),
            tool_call_id: None,
        };
        *active = Some(segments.len());
        segments.push(segment.clone());
        segment
    }
}

/// Phase 2: run one turn against a persistent live session, reusing the conversation's existing
/// session, resuming a persisted one after a restart, or connecting fresh. The CLI process is kept
/// alive in the registry between turns, so a reused/resumed session sends only the latest user
/// message (the server holds prior context), while a fresh session gets the full composed prompt.
#[allow(clippy::too_many_arguments)]
async fn run_persistent_pi_turn<E, C>(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation_id: &str,
    resolved_bin: &std::path::Path,
    args: &[String],
    cwd: &std::path::Path,
    base_env: &std::collections::HashMap<String, String>,
    provider: crate::beefapi::provider::EphemeralModelProvider,
    first_prompt: &str,
    reuse_prompt: &str,
    stable_instructions: &str,
    delivered_model: Option<String>,
    run_id: &str,
    message_id: &str,
    generation: u64,
    emit: &mut E,
    cancel: &C,
) -> Result<(), String>
where
    E: FnMut(UnifiedAgentEvent),
    C: Fn() -> bool,
{
    use crate::external_agents::session::live::{LiveSession, SessionCommand};
    use tokio::sync::{mpsc, oneshot};

    let cwd_str = cwd.to_string_lossy().to_string();
    let model_name = provider
        .model()
        .ok_or_else(|| "managed_provider_model_missing".to_string())?
        .to_string();
    let live_agent_key = format!("pi@beefex-managed/{model_name}");
    let (control, prompt) =
        match state.external_live_session_control(conversation_id, &live_agent_key, &cwd_str) {
            Some(control) => (control, reuse_prompt.to_string()),
            None => {
                let broker = crate::beefapi::pi_broker::PiProviderBroker::start(
                    state.http.clone(),
                    provider,
                )
                .await?;
                let mut env = base_env.clone();
                env.insert(
                    "BEEFEX_PI_BROKER_URL".to_string(),
                    broker.endpoint().to_string(),
                );
                env.insert("BEEFEX_PI_MODEL".to_string(), broker.model().to_string());
                let def = get_agent_def("pi").ok_or_else(|| "Pi runtime missing".to_string())?;
                let spawned = spawn_agent(def, resolved_bin, args, cwd, &env).await?;
                let mut diagnostic_roots = crate::diagnostics::default_private_roots();
                diagnostic_roots.push(cwd.to_path_buf());
                crate::diagnostics::record_app_event(
                    app,
                    crate::diagnostics::DiagnosticKind::PiChildLifecycle,
                    crate::diagnostics::DiagnosticLevel::Info,
                    "spawned",
                    None,
                    Some("pi_child_spawned"),
                    &diagnostic_roots,
                );
                let client = PiRpcClient::connect(spawned.child, Some(broker)).await?;
                let session_state = client.session_state().clone();
                crate::external_agents::session::save_session(
                    app,
                    &ExternalAgentSession {
                        conversation_id: conversation_id.to_string(),
                        agent_id: "pi".to_string(),
                        session_id: session_state.session_file.clone(),
                        stable_prompt_hash: Some(
                            crate::external_agents::session::stable_prompt_hash(
                                stable_instructions,
                            ),
                        ),
                        model: delivered_model,
                    },
                )?;
                let control = spawn_pi_session_actor(client);
                let _ = crate::external_agents::session::save_live_handle(
                    app,
                    conversation_id,
                    &crate::external_agents::session::LiveSessionHandle {
                        agent_id: "pi".to_string(),
                        protocol: "pi_rpc".to_string(),
                        native_id: session_state.session_file,
                        cwd: cwd_str.clone(),
                    },
                );
                state.register_external_live_session(
                    conversation_id.to_string(),
                    LiveSession {
                        control: control.clone(),
                        agent_id: live_agent_key.clone(),
                        cwd: cwd_str.clone(),
                        last_activity: std::time::Instant::now(),
                    },
                );
                (control, first_prompt.to_string())
            }
        };

    let (events_tx, mut events_rx) = mpsc::channel::<UnifiedAgentEvent>(64);
    let (ui_tx, mut ui_rx) = mpsc::channel::<PiExtensionUiExchange>(8);
    let (done_tx, done_rx) = oneshot::channel::<Result<(), String>>();
    control
        .send(SessionCommand::RunTurn {
            prompt,
            model: None,
            reasoning: None,
            events: events_tx,
            pi_extension_ui: Some(ui_tx),
            done: done_tx,
        })
        .await
        .map_err(|_| "Pi Task actor is unavailable".to_string())?;

    let mut done_rx = done_rx;
    let mut cancel_sent = false;
    loop {
        tokio::select! {
            result = &mut done_rx => {
                while let Ok(event) = events_rx.try_recv() {
                    emit(event);
                }
                let outcome = result.unwrap_or_else(|_| Err("Pi Task actor dropped".to_string()));
                let mut diagnostic_roots = crate::diagnostics::default_private_roots();
                diagnostic_roots.push(cwd.to_path_buf());
                let (level, transition, message_code) = match &outcome {
                    Ok(()) => (
                        crate::diagnostics::DiagnosticLevel::Info,
                        "settled",
                        "pi_actor_settled",
                    ),
                    Err(error) if error.contains("eof") => (
                        crate::diagnostics::DiagnosticLevel::Error,
                        "eof",
                        "pi_child_eof",
                    ),
                    Err(error) if error.contains("child_exit") => (
                        crate::diagnostics::DiagnosticLevel::Error,
                        "child_exit",
                        "pi_child_exit",
                    ),
                    Err(_) => (
                        crate::diagnostics::DiagnosticLevel::Error,
                        "failed",
                        "pi_actor_failed",
                    ),
                };
                crate::diagnostics::record_app_event(
                    app,
                    crate::diagnostics::DiagnosticKind::PiChildLifecycle,
                    level,
                    transition,
                    None,
                    Some(message_code),
                    &diagnostic_roots,
                );
                if matches!(&outcome, Err(error) if error != "cancelled") {
                    state.remove_external_live_session(conversation_id);
                }
                return outcome;
            }
            Some(event) = events_rx.recv() => emit(event),
            Some(exchange) = ui_rx.recv() => {
                let decision = handle_pi_extension_ui(
                    app,
                    state.inner(),
                    conversation_id,
                    run_id,
                    message_id,
                    generation,
                    exchange.request,
                ).await;
                let _ = exchange.response.send(decision);
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
        if !cancel_sent && cancel() {
            cancel_sent = true;
            let _ = control.send(SessionCommand::Cancel).await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_persistent_turn<E, C>(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation_id: &str,
    agent_id: &str,
    protocol: StreamFormat,
    resolved_bin: &std::path::Path,
    args: &[String],
    cwd: &std::path::Path,
    model: Option<String>,
    reasoning: Option<String>,
    sandbox: Option<String>,
    mcp_servers: Vec<AcpMcpServer>,
    first_prompt: &str,
    reuse_prompt: &str,
    emit: &mut E,
    cancel: &C,
) -> Result<(), String>
where
    E: FnMut(UnifiedAgentEvent),
    C: Fn() -> bool,
{
    use crate::external_agents::session::live::{LiveSession, SessionCommand};
    use crate::external_agents::session::{
        clear_live_handle, load_live_handle, save_live_handle, LiveSessionHandle,
    };
    use tokio::sync::{mpsc, oneshot};

    let cwd_str = cwd.to_string_lossy().to_string();
    let protocol_tag = persistent_protocol_tag(protocol);

    // 1. Reuse a live session already in the registry; 2. resume a persisted one; 3. fresh.
    let (control, prompt) =
        match state.external_live_session_control(conversation_id, agent_id, &cwd_str) {
            Some(control) => (control, reuse_prompt.to_string()),
            None => {
                let resume_native = load_live_handle(app, conversation_id)
                    .filter(|h| {
                        h.agent_id == agent_id && h.cwd == cwd_str && h.protocol == protocol_tag
                    })
                    .map(|h| h.native_id);
                let (control, native_id, resumed) = connect_persistent_session(
                    protocol,
                    resolved_bin,
                    args,
                    cwd,
                    model.as_deref(),
                    sandbox.as_deref(),
                    &mcp_servers,
                    resume_native,
                )
                .await?;
                let _ = save_live_handle(
                    app,
                    conversation_id,
                    &LiveSessionHandle {
                        agent_id: agent_id.to_string(),
                        protocol: protocol_tag.to_string(),
                        native_id,
                        cwd: cwd_str.clone(),
                    },
                );
                state.register_external_live_session(
                    conversation_id.to_string(),
                    LiveSession {
                        control: control.clone(),
                        agent_id: agent_id.to_string(),
                        cwd: cwd_str.clone(),
                        last_activity: std::time::Instant::now(),
                    },
                );
                // A resumed session already holds history → send only the latest message.
                let prompt = if resumed {
                    reuse_prompt.to_string()
                } else {
                    first_prompt.to_string()
                };
                (control, prompt)
            }
        };

    let (events_tx, mut events_rx) = mpsc::channel::<UnifiedAgentEvent>(64);
    let (done_tx, done_rx) = oneshot::channel::<Result<(), String>>();
    if control
        .send(SessionCommand::RunTurn {
            prompt,
            model,
            reasoning,
            events: events_tx,
            pi_extension_ui: None,
            done: done_tx,
        })
        .await
        .is_err()
    {
        state.remove_external_live_session(conversation_id);
        clear_live_handle(app, conversation_id);
        return Err("外部 CLI 会话已结束，请重试".to_string());
    }

    let mut done_rx = done_rx;
    let mut events_open = true;
    let mut cancel_sent = false;
    loop {
        tokio::select! {
            biased;
            result = &mut done_rx => {
                while let Ok(event) = events_rx.try_recv() {
                    emit(event);
                }
                let outcome = result.unwrap_or_else(|_| Err("session actor dropped".to_string()));
                // A non-cancel failure means the process likely died — drop the live session and
                // its persisted handle so the next turn connects fresh.
                if let Err(ref e) = outcome {
                    state.remove_external_live_session(conversation_id);
                    if e != "cancelled" {
                        clear_live_handle(app, conversation_id);
                    }
                }
                return outcome;
            }
            maybe_event = events_rx.recv(), if events_open => {
                match maybe_event {
                    Some(event) => emit(event),
                    None => events_open = false,
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {}
        }
        if !cancel_sent && cancel() {
            cancel_sent = true;
            let _ = control.send(SessionCommand::Cancel).await;
        }
    }
}

fn persistent_protocol_tag(protocol: StreamFormat) -> &'static str {
    match protocol {
        StreamFormat::CodexAppServer => "codex_app_server",
        StreamFormat::AcpJsonRpc => "acp_json_rpc",
        _ => "unknown",
    }
}

/// Connect (or resume) a persistent protocol session, returning its control channel, native id,
/// and whether a resume actually succeeded. Falls back to a fresh session if resume fails.
async fn connect_persistent_session(
    protocol: StreamFormat,
    resolved_bin: &std::path::Path,
    args: &[String],
    cwd: &std::path::Path,
    model: Option<&str>,
    sandbox: Option<&str>,
    mcp_servers: &[AcpMcpServer],
    resume_native: Option<String>,
) -> Result<
    (
        tokio::sync::mpsc::Sender<crate::external_agents::session::live::SessionCommand>,
        String,
        bool,
    ),
    String,
> {
    use crate::external_agents::session::acp::{spawn_acp_session_actor, AcpSession};
    use crate::external_agents::session::codex_app_server::{
        spawn_codex_session_actor, CodexAppServerSession,
    };

    match protocol {
        StreamFormat::CodexAppServer => {
            if let Some(tid) = resume_native.as_deref() {
                if let Ok(session) = CodexAppServerSession::connect(
                    resolved_bin,
                    args,
                    cwd,
                    model,
                    sandbox,
                    Some(tid),
                )
                .await
                {
                    let id = session.thread_id().to_string();
                    return Ok((spawn_codex_session_actor(session), id, true));
                }
            }
            let session =
                CodexAppServerSession::connect(resolved_bin, args, cwd, model, sandbox, None)
                    .await?;
            let id = session.thread_id().to_string();
            Ok((spawn_codex_session_actor(session), id, false))
        }
        StreamFormat::AcpJsonRpc => {
            if let Some(sid) = resume_native.as_deref() {
                if let Ok(session) =
                    AcpSession::connect(resolved_bin, args, cwd, model, mcp_servers, Some(sid))
                        .await
                {
                    let id = session.session_id().to_string();
                    return Ok((spawn_acp_session_actor(session), id, true));
                }
            }
            let session =
                AcpSession::connect(resolved_bin, args, cwd, model, mcp_servers, None).await?;
            let id = session.session_id().to_string();
            Ok((spawn_acp_session_actor(session), id, false))
        }
        _ => Err("protocol does not support persistent sessions".to_string()),
    }
}

fn text_phase_for_tool_count(tool_calls_len: usize) -> ChatMessageSegmentPhase {
    if tool_calls_len == 0 {
        ChatMessageSegmentPhase::Plain
    } else {
        ChatMessageSegmentPhase::ToolLoop
    }
}

fn promote_trailing_external_text_to_synthesis(segments: &mut [ChatMessageSegment]) {
    let last_tool_order = segments
        .iter()
        .filter(|segment| segment.kind == ChatMessageSegmentKind::Tool)
        .map(|segment| segment.order)
        .max();
    let Some(last_tool_order) = last_tool_order else {
        return;
    };

    if let Some(segment) = segments.iter_mut().rev().find(|segment| {
        segment.kind == ChatMessageSegmentKind::Text && segment.order > last_tool_order
    }) {
        // While streaming we conservatively label text after a tool as tool-loop output.
        // Once the turn is terminal, the trailing text is the assistant's synthesis. Marking
        // it here keeps hydration from adding message.content as a second identical segment.
        segment.phase = ChatMessageSegmentPhase::Synthesis;
    }
}

fn push_tool_segment(
    segments: &mut Vec<ChatMessageSegment>,
    segment_order: &mut u32,
    tool_call_id: &str,
) -> ChatMessageSegment {
    *segment_order += 1;
    let segment = ChatMessageSegment {
        id: format!("seg_{}", Uuid::new_v4()),
        kind: ChatMessageSegmentKind::Tool,
        phase: ChatMessageSegmentPhase::ToolLoop,
        order: *segment_order,
        step_number: None,
        round: Some(1),
        text: None,
        tool_call_id: Some(tool_call_id.to_string()),
    };
    segments.push(segment.clone());
    segment
}

fn apply_unified_event(
    app: &AppHandle,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    content: &mut String,
    reasoning: &mut String,
    raw_output: &mut String,
    tool_calls: &mut Vec<ToolCallRecord>,
    tool_map: &mut HashMap<String, usize>,
    usage: &mut Option<ModelUsage>,
    stream_error: &mut Option<String>,
    segments: &mut Vec<ChatMessageSegment>,
    segment_order: &mut u32,
    segment_tracker: &mut StreamSegmentTracker,
    cwd: &std::path::Path,
    event: UnifiedAgentEvent,
) {
    let now = Local::now().timestamp();
    match event {
        UnifiedAgentEvent::TextDelta { delta } => {
            content.push_str(&delta);
            let segment = segment_tracker.append(
                ChatMessageSegmentKind::Text,
                segments,
                segment_order,
                tool_calls.len(),
                &delta,
            );
            emit_chat_stream_delta(
                app,
                conversation_id,
                run_id,
                message_id,
                &delta,
                None,
                Some(&segment),
            );
        }
        UnifiedAgentEvent::ThinkingDelta { delta } => {
            reasoning.push_str(&delta);
            let segment = segment_tracker.append(
                ChatMessageSegmentKind::Reasoning,
                segments,
                segment_order,
                tool_calls.len(),
                &delta,
            );
            emit_chat_stream_delta(
                app,
                conversation_id,
                run_id,
                message_id,
                "",
                Some(&delta),
                Some(&segment),
            );
        }
        UnifiedAgentEvent::ToolUse { id, name, input } => {
            segment_tracker.reset_text();
            segment_tracker.reset_reasoning();
            let segment = push_tool_segment(segments, segment_order, &id);
            emit_chat_stream_delta(
                app,
                conversation_id,
                run_id,
                message_id,
                "",
                None,
                Some(&segment),
            );
            let structured_content = pi_tool_initial_receipt_context(&name, &input, cwd);
            let record = ToolCallRecord {
                id: id.clone(),
                name: name.clone(),
                source: "external_cli".to_string(),
                server_id: None,
                arguments: input.to_string(),
                status: ToolCallStatus::Running,
                result_preview: None,
                error: None,
                duration_ms: None,
                started_at: Some(now),
                completed_at: None,
                round: 1,
                sensitive: false,
                artifacts: vec![],
                trace_id: None,
                span_id: None,
                structured_content,
            };
            tool_map.insert(id.clone(), tool_calls.len());
            tool_calls.push(record.clone());
            emit_chat_tool_record(app, conversation_id, run_id, message_id, &record);
        }
        UnifiedAgentEvent::ToolResult {
            tool_use_id,
            content: result_content,
            is_error,
        } => {
            if let Some(idx) = tool_map.get(&tool_use_id).copied() {
                if let Some(record) = tool_calls.get_mut(idx) {
                    record.status = if is_error {
                        ToolCallStatus::Error
                    } else {
                        ToolCallStatus::Success
                    };
                    record.result_preview = Some(truncate_for_preview(&result_content, 800));
                    record.completed_at = Some(now);
                    emit_chat_tool_record(app, conversation_id, run_id, message_id, record);
                }
            }
        }
        UnifiedAgentEvent::PiToolResult {
            tool_use_id,
            content: result_content,
            is_error,
            result,
        } => {
            if let Some(idx) = tool_map.get(&tool_use_id).copied() {
                if let Some(record) = tool_calls.get_mut(idx) {
                    record.status = if is_error {
                        ToolCallStatus::Error
                    } else {
                        ToolCallStatus::Success
                    };
                    record.result_preview = Some(truncate_for_preview(&result_content, 800));
                    record.completed_at = Some(now);
                    record.structured_content = pi_tool_receipt_content(record, cwd, &result);
                    emit_chat_tool_record(app, conversation_id, run_id, message_id, record);
                }
            }
        }
        UnifiedAgentEvent::Usage { usage: u } => {
            *usage = Some(u);
        }
        UnifiedAgentEvent::Error { message, .. } => {
            eprintln!("[external-agent] stream error: {message}");
            *stream_error = Some(message);
        }
        UnifiedAgentEvent::Raw { line } => {
            // Unparsed stdout line — accumulate (capped) as a fallback surfaced only if the run
            // produced no structured content.
            if !raw_output.is_empty() {
                raw_output.push('\n');
            }
            raw_output.push_str(&line);
            if raw_output.chars().count() > 8192 {
                *raw_output = tail_chars(raw_output, 8192);
            }
        }
        UnifiedAgentEvent::RuntimeStatus { kind, data } => {
            let _ = app.emit(
                "chat-pi-runtime-status",
                serde_json::json!({
                    "conversationId": conversation_id,
                    "runId": run_id,
                    "kind": kind,
                    "data": data,
                }),
            );
        }
        _ => {}
    }
}

fn pi_tool_receipt_content(
    record: &ToolCallRecord,
    cwd: &std::path::Path,
    result: &serde_json::Value,
) -> Option<serde_json::Value> {
    let input = serde_json::from_str::<serde_json::Value>(&record.arguments).ok()?;
    match record.name.as_str() {
        "bash" => Some(serde_json::json!({
            "type": "command_execution",
            "command": input.get("command").and_then(serde_json::Value::as_str)?,
            "cwd": cwd.to_string_lossy(),
            "exit_status": 0,
            "stdout": record.result_preview.as_deref().unwrap_or_default(),
            "pi_result": result,
        })),
        "edit" => {
            let path = input.get("path").and_then(serde_json::Value::as_str)?;
            let patch = result
                .get("details")
                .and_then(|details| details.get("patch"))
                .and_then(serde_json::Value::as_str)?;
            if patch.trim().is_empty() {
                return None;
            }
            let additions = patch
                .lines()
                .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
                .count();
            let removals = patch
                .lines()
                .filter(|line| line.starts_with('-') && !line.starts_with("---"))
                .count();
            Some(serde_json::json!({
                "type": "file_edit",
                "files": [{
                    "path": path,
                    "additions": additions,
                    "removals": removals,
                    "diff": patch,
                }],
                "pi_result": result,
            }))
        }
        "write" => {
            let path = input.get("path").and_then(serde_json::Value::as_str)?;
            let content = input.get("content").and_then(serde_json::Value::as_str)?;
            let target = pi_tool_target_path(cwd, path)?;
            let created_new = record
                .structured_content
                .as_ref()
                .and_then(|value| value.get("target_existed"))
                .and_then(serde_json::Value::as_bool)
                == Some(false);
            let readback = std::fs::read_to_string(&target).ok()?;
            if !created_new || readback != content {
                return Some(serde_json::json!({
                    "type": "pi_tool_result",
                    "input": input,
                    "result": result,
                }));
            }
            let receipt_path = target
                .strip_prefix(cwd)
                .unwrap_or(&target)
                .to_string_lossy()
                .to_string();
            let mut patch = format!("--- /dev/null\n+++ b/{receipt_path}\n");
            let additions = content.lines().count().max(1);
            patch.push_str(&format!("@@ -0,0 +1,{additions} @@\n"));
            for line in content.lines() {
                patch.push('+');
                patch.push_str(line);
                patch.push('\n');
            }
            if !content.ends_with('\n') {
                patch.push_str("\\ No newline at end of file\n");
            }
            Some(serde_json::json!({
                "type": "file_write",
                "files": [{
                    "path": receipt_path,
                    "additions": additions,
                    "removals": 0,
                    "diff": patch,
                }],
                "pi_result": result,
            }))
        }
        _ => Some(serde_json::json!({
            "type": "pi_tool_result",
            "input": input,
            "result": result,
        })),
    }
}

fn pi_tool_initial_receipt_context(
    name: &str,
    input: &serde_json::Value,
    cwd: &std::path::Path,
) -> Option<serde_json::Value> {
    if name != "write" {
        return Some(input.clone());
    }
    let path = input.get("path").and_then(serde_json::Value::as_str)?;
    let target = pi_tool_target_path(cwd, path)?;
    Some(serde_json::json!({
        "type": "pi_write_pending",
        "target_existed": target.exists(),
    }))
}

fn pi_tool_target_path(cwd: &std::path::Path, path: &str) -> Option<std::path::PathBuf> {
    let candidate = std::path::Path::new(path);
    let target = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd.join(candidate)
    };
    target.starts_with(cwd).then_some(target)
}

fn truncate_for_preview(value: &str, max_chars: usize) -> String {
    let mut out: String = value.chars().take(max_chars).collect();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_rpc_disables_the_kivio_host_skill_bridge() {
        assert!(!host_skill_bridge_enabled(StreamFormat::PiRpc));
        assert!(host_skill_bridge_enabled(StreamFormat::ClaudeStreamJson));
    }

    #[test]
    fn managed_pi_runtime_isolates_global_home_skills() {
        let root =
            std::env::temp_dir().join(format!("beefex-managed-pi-home-{}", uuid::Uuid::new_v4()));
        let env = managed_pi_runtime_env(&root).expect("managed Pi env");
        assert_eq!(
            env.get("HOME"),
            Some(&root.join("home").to_string_lossy().into_owned())
        );
        assert_eq!(
            env.get("PI_CODING_AGENT_DIR"),
            Some(&root.join("agent").to_string_lossy().into_owned())
        );
        assert!(!root.join("home/.agents/skills").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(root.join("home"))
                    .expect("home metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        std::fs::remove_dir_all(root).expect("remove managed Pi test root");
    }

    #[test]
    fn pi_extension_approval_extracts_tool_fields_without_internal_envelope() {
        let message = serde_json::json!({
            "toolCallId": "tool-123",
            "toolName": "write",
            "input": {"path": "out.txt", "content": "ok\n"},
            "projectRoot": "/tmp/project"
        })
        .to_string();

        let (name, arguments) = pi_extension_tool_record_fields(&message);

        assert_eq!(name, "write");
        assert_eq!(arguments["path"], "out.txt");
        assert!(arguments.get("toolCallId").is_none());
        assert!(arguments.get("projectRoot").is_none());
    }

    fn pi_record(name: &str, arguments: serde_json::Value) -> ToolCallRecord {
        ToolCallRecord {
            id: format!("pi-{name}"),
            name: name.to_string(),
            source: "external_cli".to_string(),
            server_id: None,
            arguments: arguments.to_string(),
            status: ToolCallStatus::Success,
            result_preview: Some("ok".to_string()),
            error: None,
            duration_ms: None,
            started_at: Some(1),
            completed_at: Some(2),
            round: 1,
            sensitive: true,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: None,
        }
    }

    #[test]
    fn stream_segment_tracker_reuses_text_segment_for_deltas() {
        let mut segments = Vec::new();
        let mut order = 0u32;
        let mut tracker = StreamSegmentTracker::default();

        let first = tracker.append(
            ChatMessageSegmentKind::Text,
            &mut segments,
            &mut order,
            0,
            "你",
        );
        let second = tracker.append(
            ChatMessageSegmentKind::Text,
            &mut segments,
            &mut order,
            0,
            "好",
        );

        assert_eq!(segments.len(), 1);
        assert_eq!(first.id, second.id);
        assert_eq!(segments[0].text.as_deref(), Some("你好"));
        assert_eq!(segments[0].phase, ChatMessageSegmentPhase::Plain);
    }

    #[test]
    fn push_tool_segment_increments_order_and_sets_tool_kind() {
        let mut segments = Vec::new();
        let mut order = 2u32;
        let first = push_tool_segment(&mut segments, &mut order, "tool-1");
        let second = push_tool_segment(&mut segments, &mut order, "tool-2");

        assert_eq!(segments.len(), 2);
        assert_eq!(first.kind, ChatMessageSegmentKind::Tool);
        assert_eq!(first.order, 3);
        assert_eq!(first.tool_call_id.as_deref(), Some("tool-1"));
        assert_eq!(second.order, 4);
        assert_eq!(second.phase, ChatMessageSegmentPhase::ToolLoop);
    }

    #[test]
    fn stream_segment_tracker_starts_new_text_segment_after_tool_use() {
        let mut segments = Vec::new();
        let mut order = 0u32;
        let mut tracker = StreamSegmentTracker::default();

        tracker.append(
            ChatMessageSegmentKind::Text,
            &mut segments,
            &mut order,
            0,
            "before",
        );
        tracker.reset_text();
        let after = tracker.append(
            ChatMessageSegmentKind::Text,
            &mut segments,
            &mut order,
            1,
            "after",
        );

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text.as_deref(), Some("before"));
        assert_eq!(segments[1].text.as_deref(), Some("after"));
        assert_eq!(after.phase, ChatMessageSegmentPhase::ToolLoop);
    }

    #[test]
    fn terminal_external_text_after_tool_becomes_synthesis() {
        let mut segments = vec![
            ChatMessageSegment {
                id: "tool".to_string(),
                kind: ChatMessageSegmentKind::Tool,
                phase: ChatMessageSegmentPhase::ToolLoop,
                order: 1,
                step_number: None,
                round: Some(1),
                text: None,
                tool_call_id: Some("call-1".to_string()),
            },
            ChatMessageSegment {
                id: "answer".to_string(),
                kind: ChatMessageSegmentKind::Text,
                phase: ChatMessageSegmentPhase::ToolLoop,
                order: 2,
                step_number: None,
                round: None,
                text: Some("done".to_string()),
                tool_call_id: None,
            },
        ];

        promote_trailing_external_text_to_synthesis(&mut segments);

        assert_eq!(segments[1].phase, ChatMessageSegmentPhase::Synthesis);
    }

    #[test]
    fn pi_edit_receipt_requires_an_observed_patch() {
        let mut record = pi_record("edit", serde_json::json!({ "path": "src/demo.ts" }));
        record.structured_content = pi_tool_receipt_content(
            &record,
            std::path::Path::new("/workspace/beefex"),
            &serde_json::json!({
                "details": {
                    "patch": "--- a/src/demo.ts\n+++ b/src/demo.ts\n@@ -0,0 +1 @@\n+export const ok = true\n"
                }
            }),
        );
        let receipt = crate::chat::receipt::build_completion_receipt(&[record]);
        assert_eq!(receipt.changed_files.len(), 1);
        assert_eq!(receipt.changed_files[0].path, "src/demo.ts");
        assert_eq!(receipt.changed_files[0].additions, 1);
        assert!(receipt.changed_files[0].has_diff);
    }

    #[test]
    fn pi_write_receipt_requires_new_file_readback_before_emitting_diff() {
        let temp =
            std::env::temp_dir().join(format!("beefex-pi-write-receipt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        let target = temp.join("created.txt");
        let input = serde_json::json!({
            "path": target,
            "content": "created-by-pi"
        });
        let mut record = pi_record("write", input.clone());
        record.arguments = input.to_string();
        record.structured_content = pi_tool_initial_receipt_context("write", &input, &temp);
        std::fs::write(&target, "created-by-pi").unwrap();
        record.structured_content = pi_tool_receipt_content(
            &record,
            &temp,
            &serde_json::json!({"content":[{"type":"text","text":"ok"}]}),
        );

        let receipt = crate::chat::receipt::build_completion_receipt(&[record]);
        assert_eq!(receipt.changed_files.len(), 1);
        assert_eq!(receipt.changed_files[0].path, "created.txt");
        assert_eq!(receipt.changed_files[0].additions, 1);
        assert!(receipt.changed_files[0].has_diff);
        std::fs::remove_file(target).unwrap();
        std::fs::remove_dir(temp).unwrap();
    }

    #[test]
    fn pi_bash_receipt_uses_observed_output_and_project_cwd() {
        let mut record = pi_record("bash", serde_json::json!({ "command": "npm test" }));
        record.result_preview = Some("9 passed".to_string());
        record.structured_content = pi_tool_receipt_content(
            &record,
            std::path::Path::new("/workspace/beefex"),
            &serde_json::json!({ "content": [{ "type": "text", "text": "9 passed" }] }),
        );
        let receipt = crate::chat::receipt::build_completion_receipt(&[record]);
        assert_eq!(receipt.commands.len(), 1);
        assert_eq!(receipt.commands[0].command, "npm test");
        assert_eq!(receipt.commands[0].cwd, "/workspace/beefex");
        assert_eq!(receipt.commands[0].stdout, "9 passed");
    }
}
