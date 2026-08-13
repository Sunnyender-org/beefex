use crate::chat::{ChatCompletionReceipt, ToolCallRecord};

pub fn build_completion_receipt(records: &[ToolCallRecord]) -> ChatCompletionReceipt {
    let changed_files = records
        .iter()
        .filter(|record| matches!(record.status, crate::chat::ToolCallStatus::Success))
        .filter_map(|record| record.structured_content.as_ref())
        .flat_map(|content| {
            content
                .get("files")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|file| {
            let diff = file.get("diff").and_then(serde_json::Value::as_str)?;
            if diff.trim().is_empty() {
                return None;
            }
            Some(crate::chat::ChatChangedFileReceipt {
                path: file
                    .get("path")
                    .and_then(serde_json::Value::as_str)?
                    .to_string(),
                additions: file
                    .get("additions")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default() as usize,
                removals: file
                    .get("removals")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default() as usize,
                has_diff: true,
            })
        })
        .collect::<Vec<_>>();
    let commands = records
        .iter()
        .filter(|record| matches!(record.status, crate::chat::ToolCallStatus::Success))
        .filter_map(|record| record.structured_content.as_ref())
        .filter(|content| {
            content.get("type").and_then(serde_json::Value::as_str) == Some("command_execution")
        })
        .filter_map(|content| {
            Some(crate::chat::ChatCommandReceipt {
                command: content
                    .get("command")
                    .and_then(serde_json::Value::as_str)?
                    .to_string(),
                cwd: content
                    .get("cwd")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(".")
                    .to_string(),
                exit_status: content
                    .get("exit_status")
                    .and_then(serde_json::Value::as_i64),
                stdout: crate::chat::agent::execute::truncate_chars(
                    content
                        .get("stdout")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default(),
                    4_000,
                ),
            })
        })
        .collect::<Vec<_>>();
    let validations = commands
        .iter()
        .filter(|command| command.exit_status == Some(0) && is_validation_command(&command.command))
        .map(|command| command.command.clone())
        .collect();
    ChatCompletionReceipt {
        changed_files,
        commands,
        validations,
    }
}

fn is_validation_command(command: &str) -> bool {
    let command = command.to_ascii_lowercase();
    [
        " test", "test ", " check", "check ", " lint", "lint ", " build", "build ", " vet", "vet ",
    ]
    .iter()
    .any(|marker| command.contains(marker))
        || ["test", "check", "lint", "build", "vet"]
            .iter()
            .any(|marker| command == *marker)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ToolCallRecord, ToolCallStatus};

    fn record(name: &str, structured_content: serde_json::Value) -> ToolCallRecord {
        ToolCallRecord {
            id: format!("tool-{name}"),
            name: name.to_string(),
            source: "native".to_string(),
            server_id: None,
            arguments: "{}".to_string(),
            status: ToolCallStatus::Success,
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

    #[test]
    fn receipt_lists_only_files_with_observed_non_empty_diffs() {
        let changed = record(
            "edit",
            serde_json::json!({
                "target_touched": true,
                "files": [{
                    "path": "src/demo.ts",
                    "additions": 1,
                    "removals": 0,
                    "diff": "--- a/src/demo.ts\n+++ b/src/demo.ts\n+export const ok = true"
                }]
            }),
        );
        let no_diff = record(
            "write",
            serde_json::json!({
                "target_touched": false,
                "files": [{"path": "README.md", "additions": 0, "removals": 0, "diff": ""}]
            }),
        );

        let receipt = build_completion_receipt(&[changed, no_diff]);

        assert_eq!(receipt.changed_files.len(), 1);
        assert_eq!(receipt.changed_files[0].path, "src/demo.ts");
        assert_eq!(receipt.changed_files[0].additions, 1);
        assert!(receipt.changed_files[0].has_diff);
    }

    #[test]
    fn receipt_records_command_cwd_exit_stdout_and_validation() {
        let command = record(
            "bash",
            serde_json::json!({
                "type": "command_execution",
                "command": "bun test",
                "cwd": ".",
                "exit_status": 0,
                "stdout": "12 passed\n"
            }),
        );

        let receipt = build_completion_receipt(&[command]);

        assert_eq!(receipt.commands.len(), 1);
        assert_eq!(receipt.commands[0].command, "bun test");
        assert_eq!(receipt.commands[0].cwd, ".");
        assert_eq!(receipt.commands[0].exit_status, Some(0));
        assert_eq!(receipt.commands[0].stdout, "12 passed\n");
        assert_eq!(receipt.validations, vec!["bun test"]);
    }
}
