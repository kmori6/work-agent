# Agent Process Flow

See [architecture.md](architecture.md) for component responsibilities.

## Main Flow

```mermaid
flowchart TD
    A["CLI receives user message"] --> B["AgentUsecase::handle"]
    B --> C{"Pending approval for session?"}
    C -- "Yes" --> C1["Return ApprovalPending error"]
    C -- "No" --> D["Load chat history"]
    D --> E["Load latest token usage"]
    E --> F["Build context with ContextService"]
    F --> G["Save user message"]
    G --> H["Load tool_execution_rules"]
    H --> I["AgentService::run"]

    I --> J["Build system + context + user messages"]
    J --> K["Agent loop"]
    K --> L["Call LLM with tool specs"]
    L --> M{"LLM returned tool calls?"}

    M -- "No" --> N["Create final assistant message"]
    N --> O["Return AgentOutput::Completed"]
    O --> P["Usecase saves turn messages and token usage"]
    P --> Q["Return assistant message to CLI"]

    M -- "Yes" --> R["Add assistant tool-call message to turn messages"]
    R --> S["Plan tool-call batch"]
    S --> T{"Decision per tool call"}

    T -- "Allow" --> U["Plan Run"]
    T -- "Deny or unknown tool" --> V["Plan Block error result"]
    T -- "Ask" --> W["Stop planning at this call"]

    U --> S
    V --> S
    W --> X["Execute planned calls before approval"]

    X --> Y["Run consecutive Run calls in parallel"]
    X --> Z["Convert Block calls to error tool results"]
    Y --> AA["Accumulate tool results"]
    Z --> AA

    AA --> AB["Build AgentApprovalRequest"]
    AB --> AC["Return AgentOutput::ApprovalRequested"]
    AC --> AD["Usecase stores pending approval in memory"]
    AD --> AE["CLI shows /approve or /deny prompt"]

    S --> AF{"No approval needed?"}
    AF -- "Yes" --> AG["Execute planned calls"]
    AG --> AH["Append tool results to LLM context"]
    AH --> K
```

## Tool Execution Decisions

| Tool policy        | Stored rule       | Decision                                        |
| ------------------ | ----------------- | ----------------------------------------------- |
| `Auto`             | none              | `Allow`                                         |
| `Ask`              | none              | `Ask`                                           |
| `ConfirmEveryTime` | any non-deny rule | `Ask`                                           |
| any policy         | `allow`           | `Allow` (except `ConfirmEveryTime` stays `Ask`) |
| any policy         | `ask`             | `Ask`                                           |
| any policy         | `deny`            | `Deny`                                          |

Denied and unknown tools return an error result to the LLM without executing.

## Planned Tool Calls

Each tool call in a batch is planned as one of:

- `Run` — execute
- `Block` — return error result without executing
- `PendingToolApproval` — pause loop before this call

Execution preserves order while parallelizing safe calls:

```
Run, Run, Block, Run  →  parallel(Run, Run), Block result, parallel(Run)
```

## Approval Request Flow

1. Runnable calls before the approval point are executed.
2. Results are stored in `AgentApprovalRequest.accumulated_tool_results`.
3. The approval target and remaining calls are stored in memory keyed by `session_id`.
4. CLI shows `/approve` or `/deny` prompt.

## Approve Flow

1. Approval decision written to `tool_call_approvals`.
2. Current `tool_execution_rules` reloaded.
3. Pending tool re-checked; executed if allowed, error result if denied.
4. Remaining tool calls processed with current rules.
5. Turn messages and token usage saved on completion.
6. Pending approval cleared after successful save.

## Deny Flow

1. Denial decision written to `tool_call_approvals`.
2. Accumulated tool results, denied tool result, and skipped tool results persisted.
3. Assistant denial message saved.
4. Pending approval cleared.

## Persistence

| What                   | Where                  |
| ---------------------- | ---------------------- |
| Chat messages          | `chat_messages`        |
| Token usage            | `token_usages`         |
| Approval decisions     | `tool_call_approvals`  |
| Tool execution rules   | `tool_execution_rules` |
| Pending approval state | In-memory only         |
