# Tools

All registered tools are wired in [`src/main.rs`](../src/main.rs).

## Workspace

| Tool          | Description                                                           |
| ------------- | --------------------------------------------------------------------- |
| `file_search` | Find files by glob pattern                                            |
| `file_read`   | Read a file; binary files (PDF, DOCX, etc.) are converted to Markdown |
| `text_search` | Search text across workspace files                                    |
| `file_write`  | Write full content to a file (overwrites)                             |
| `file_edit`   | Replace an exact text block in a file                                 |
| `shell_exec`  | Run a non-interactive shell command                                   |

## Web & Research

| Tool         | Description                          |
| ------------ | ------------------------------------ |
| `web_search` | Search the public web                |
| `web_fetch`  | Fetch and extract content from a URL |
| `research`   | LLM-assisted deep research workflow  |

## Multimodal

| Tool  | Description                              |
| ----- | ---------------------------------------- |
| `asr` | Transcribe speech/audio to text          |
| `ocr` | Extract text from images or scanned PDFs |

## Execution Policy

Each tool has a default `ToolExecutionPolicy`:

| Policy             | Behavior                                                                |
| ------------------ | ----------------------------------------------------------------------- |
| `Auto`             | Runs automatically                                                      |
| `Ask`              | Pauses and requests user approval                                       |
| `ConfirmEveryTime` | Always requests approval; cannot be overridden by a stored `allow` rule |

Defaults:

- Read/search/extraction tools: `Auto`
- `shell_exec`, `file_write`, `file_edit`: `Ask`

Persisted rules in `tool_execution_rules` (`allow` / `ask` / `deny`) are combined with the tool's default policy at runtime to produce the final decision.

Unknown tool calls return an error result to the LLM without executing.

## Approval Commands

| Command    | Behavior                                                                  |
| ---------- | ------------------------------------------------------------------------- |
| `/approve` | Records approval, rechecks tool against current rules, resumes agent loop |
| `/deny`    | Records denial, marks tool as denied, skips remaining tools, ends turn    |

Pending approval state is in-memory only.
