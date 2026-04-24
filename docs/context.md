# Context Management

Each agent turn loads the full chat history and passes it to `ContextService`, which compacts it if needed before sending to the LLM.

## Parameters

| Parameter                      | Default   | Description                                                    |
| ------------------------------ | --------- | -------------------------------------------------------------- |
| `context_window_tokens`        | 1,000,000 | Maximum token capacity of the context window                   |
| `compaction_threshold_percent` | 80%       | Compaction triggers when usage exceeds this threshold          |
| `recent_messages_to_keep`      | 8         | Number of recent messages preserved verbatim during compaction |

## Compaction

Triggered when previous turn's token usage ≥ 80% of the context window:

1. Split history: old messages (all but last 8) + recent messages (last 8).
2. Summarize old messages via LLM, retaining facts, decisions, open tasks, filenames, and implementation direction.
3. Replace old messages with a single summary message.
4. Prepend summary to recent messages.

Compaction applies to the current turn only. The full history remains in the database.

## Notes

- Token usage from the **previous turn** is used to decide whether to compact.
- Tool calls and results are summarized as simplified text (tool names and text only).
- Attachments appear in the summary as filenames.
- Long-term memory across sessions is not yet implemented (see future `memory.md`).
