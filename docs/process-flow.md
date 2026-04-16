# Agent Process Flow

This document describes the current processing flow at a high level.

## Main Flow

```mermaid
flowchart TD
    A[Initialize conversation state]
    B[Receive user message]
    C[Build prompt and working context]
    D[Enter agent loop]
    E[Call the language model]
    F[Store model output in conversation history]
    G{Did the model request any tools?}
    H[Generate final response]
    I[Return response to the user]
    J[Resolve requested tools one by one]
    K{Was the tool request valid?}
    L[Create tool error result]
    M[Execute the requested tool]
    N{Did tool execution succeed?}
    O[Create tool success result]
    P[Create tool failure result]
    Q[Append tool result to conversation history]
    R{Reached the iteration limit?}
    S[Return iteration-limit error]

    A --> B --> C --> D --> E --> F --> G

    G -- No --> H --> I

    G -- Yes --> J --> K
    K -- No --> L --> Q
    K -- Yes --> M --> N
    N -- Yes --> O --> Q
    N -- No --> P --> Q

    Q --> R
    R -- No --> E
    R -- Yes --> S
```

## Notes

- The flow starts from a user message and ends when the agent can return a final response.
- If the model requests tools, their results are added back into the conversation before the next model call.
- Tool handling is sequential in the current implementation.
- If the loop exceeds the allowed number of iterations, the process ends with an error.
