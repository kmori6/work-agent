# Tools

This document gives a short overview of the tools currently registered in `work-agent`.
The current source of truth is [src/main.rs](../src/main.rs).

## Workspace tools

- `file_search`: Find files in the workspace by glob-style path pattern.
- `file_read`: Read a file from the workspace. Text files are returned directly, and supported binary files such as PDF or Office documents are converted to Markdown. Optional line-range arguments can narrow the returned slice.
- `text_search`: Search text across workspace files.
- `file_write`: Write full UTF-8 text content to a file in the workspace. Parent directories are created automatically, and existing files are replaced.
- `file_edit`: Replace exactly one matched text block in a UTF-8 file in the workspace. Use `file_write` for full rewrites.
- `shell_exec`: Run one non-interactive shell command in the workspace, with optional `workdir`.

## Web and research tools

- `web_search`: Search the public web.
- `web_fetch`: Fetch and extract content from a web page.
- `research`: Higher-level web research workflow built on top of LLM-assisted search/fetch.

## Multimodal tools

- `asr`: Convert speech/audio input into text.
- `ocr`: Extract text from local images or PDF files.

## Notes

- The current workspace editing surface is intentionally split into small responsibilities: `file_write` handles full rewrites, `file_edit` handles exact one-shot replacements, and `shell_exec` stays a separate fallback for command execution.
- The current tool surface is strongest for workspace inspection, text operations, lightweight web access, and basic multimodal extraction.
- `shell_exec` is available now, but it should still be hardened further for safer command policy and execution boundaries.
- `research` is useful today, but it is closer to a workflow/skill than a low-level primitive tool.
