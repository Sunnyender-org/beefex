use crate::chat::Conversation;

pub struct ComposedExternalPrompt {
    pub full_prompt: String,
    pub instructions_block: String,
}

pub fn is_cli_slash_input(content: &str) -> bool {
    content.trim_start().starts_with('/')
}

pub fn compose_external_prompt_passthrough(latest_user_message: &str) -> ComposedExternalPrompt {
    ComposedExternalPrompt {
        full_prompt: latest_user_message.trim().to_string(),
        instructions_block: String::new(),
    }
}

pub fn compose_external_prompt(
    conversation: &Conversation,
    daemon_instructions: &str,
    skill_body: Option<&str>,
    skip_instructions: bool,
    skip_transcript: bool,
    latest_user_message: &str,
) -> ComposedExternalPrompt {
    let skill_section = skill_body.unwrap_or_default().to_string();

    let mut instructions_parts = Vec::new();
    if !skip_instructions {
        if !daemon_instructions.trim().is_empty() {
            instructions_parts.push(daemon_instructions.trim().to_string());
        }
        if !skill_section.trim().is_empty() {
            instructions_parts.push(skill_section);
        }
    }

    let instructions_block = instructions_parts.join("\n\n---\n\n");

    let transcript = if skip_transcript {
        String::new()
    } else {
        build_transcript_before_latest(conversation, latest_user_message)
    };

    let mut full = String::new();
    if !instructions_block.is_empty() {
        full.push_str("# Instructions (read first)\n\n");
        full.push_str(&instructions_block);
        full.push_str("\n\n---\n\n");
    }
    full.push_str("# User request\n\n");
    if !transcript.is_empty() {
        full.push_str(&transcript);
        full.push('\n');
    }
    full.push_str(latest_user_message.trim());

    ComposedExternalPrompt {
        full_prompt: full,
        instructions_block,
    }
}

fn build_transcript_before_latest(
    conversation: &Conversation,
    latest_user_message: &str,
) -> String {
    let mut lines = Vec::new();
    let latest_index = conversation.messages.iter().rposition(|message| {
        message.role == "user" && message.content.trim() == latest_user_message.trim()
    });
    for (index, message) in conversation.messages.iter().enumerate() {
        if Some(index) == latest_index {
            continue;
        }
        let role = message.role.as_str();
        let label = match role {
            "user" => "user",
            "assistant" => "assistant",
            _ => continue,
        };
        let text = message.content.trim();
        if text.is_empty() {
            continue;
        }
        lines.push(format!("## {label}\n{text}"));
    }
    lines.join("\n\n")
}

pub fn cwd_hint(cwd: &str) -> String {
    format!("Your working directory is `{cwd}`.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::{
        AgentPlanState, AgentRuntimeConfig, AgentTodoState, Conversation, ConversationContextState,
    };

    fn empty_conversation() -> Conversation {
        Conversation {
            id: "c1".to_string(),
            title: "t".to_string(),
            provider_id: "p".to_string(),
            model: "m".to_string(),
            messages: vec![],
            agent_runtime: AgentRuntimeConfig::default(),
            active_skill_id: None,
            assistant_id: None,
            assistant_snapshot: None,
            created_at: 0,
            updated_at: 0,
            pinned: false,
            folder: None,
            project_id: None,
            set_id: None,
            context_state: ConversationContextState::default(),
            agent_todo_state: AgentTodoState::default(),
            agent_plan_state: AgentPlanState::default(),
            knowledge_base_ids: Vec::new(),
            force_knowledge_search: false,
            thinking_level: None,
            reply_models: Vec::new(),
            group_selections: std::collections::HashMap::new(),
            forked_from: None,
            last_run_status: None,
            last_run_id: None,
            last_run_terminal_at: None,
            last_run_receipt: None,
        }
    }

    #[test]
    fn compose_includes_instructions_and_user_request() {
        let conv = empty_conversation();
        let composed = compose_external_prompt(
            &conv,
            "system rules",
            Some("skill body"),
            false,
            true,
            "hello",
        );
        assert!(composed.full_prompt.contains("# Instructions"));
        assert!(composed.full_prompt.contains("skill body"));
        assert!(composed.full_prompt.contains("hello"));
    }

    #[test]
    fn compose_does_not_duplicate_latest_user_message() {
        let mut conv = empty_conversation();
        conv.messages.push(crate::chat::ChatMessage {
            id: "u1".to_string(),
            role: "user".to_string(),
            content: "hello once".to_string(),
            attachments: vec![],
            reasoning: None,
            artifacts: vec![],
            tool_calls: vec![],
            segments: vec![],
            agent_plan: None,
            api_messages: vec![],
            model_messages: vec![],
            active_skill_id: None,
            run_entry: None,
            stream_outcome: None,
            usage: None,
            anchor_usage: None,
            group_id: None,
            provider_id: None,
            model: None,
            timestamp: 0,
        });
        let composed =
            compose_external_prompt(&conv, "system rules", None, false, false, "hello once");
        assert_eq!(composed.full_prompt.matches("hello once").count(), 1);
    }

    #[test]
    fn is_cli_slash_input_detects_leading_slash() {
        assert!(is_cli_slash_input("/compact"));
        assert!(is_cli_slash_input("  /model gpt-5"));
        assert!(!is_cli_slash_input("hello /compact"));
        assert!(!is_cli_slash_input("plain text"));
    }

    #[test]
    fn passthrough_prompt_is_raw_slash_without_wrapper() {
        let composed = compose_external_prompt_passthrough("  /model gpt-5  ");
        assert_eq!(composed.full_prompt, "/model gpt-5");
        assert!(composed.instructions_block.is_empty());
        assert!(!composed.full_prompt.contains("# Instructions"));
    }

    #[test]
    fn pi_native_prompt_hint_does_not_mention_legacy_skill_staging() {
        let hint = cwd_hint("/tmp/project");
        assert_eq!(hint, "Your working directory is `/tmp/project`.");
        assert!(!hint.contains(".kivio"));
        assert!(!hint.contains("skills-staged"));
    }
}
