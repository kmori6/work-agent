# Tools

This document gives a short overview of the tools currently registered in `work-agent`.
The current source of truth is [src/main.rs](../src/main.rs).

## Workspace tools

- `file_search`: Find files in the workspace by glob-style path pattern.
- `read_file`: Read a text file from the workspace.
- `text_search`: Search text across workspace files.
- `text_file_write`: Create a new UTF-8 text file in the workspace.
- `text_file_edit`: Edit a UTF-8 text file by replacing an exact text match.
- `shell_exec`: Run one non-interactive shell command in the workspace, with optional `workdir`.

## Web and research tools

- `web_search`: Search the public web.
- `web_fetch`: Fetch and extract content from a web page.
- `research`: Higher-level web research workflow built on top of LLM-assisted search/fetch.

## Multimodal tools

- `asr`: Convert speech/audio input into text.
- `ocr`: Extract text from local images or PDF files.

## Notes

- The current tool surface is strongest for workspace inspection, text operations, lightweight web access, and basic multimodal extraction.
- `shell_exec` is available now, but it should still be hardened further for safer command policy and execution boundaries.
- `research` is useful today, but it is closer to a workflow/skill than a low-level primitive tool.
