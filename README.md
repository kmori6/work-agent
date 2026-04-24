# 🦉 Commander

AI agent for R&D software engineering work

[![Test](https://github.com/kmori6/commander/actions/workflows/test.yaml/badge.svg)](https://github.com/kmori6/commander/actions/workflows/test.yaml)
[![Lint](https://github.com/kmori6/commander/actions/workflows/lint.yaml/badge.svg)](https://github.com/kmori6/commander/actions/workflows/lint.yaml)

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) 1.85+
- [Docker](https://docs.docker.com/get-docker/)
- [markitdown](https://github.com/microsoft/markitdown) (`pip install markitdown`)
- AWS account with Bedrock access
- [Tavily](https://tavily.com/) API key

## Setup

1. Start PostgreSQL and run migrations:

```bash
docker compose up -d postgres flyway-admin flyway-agent
```

2. Copy `.env.sample` to `.env` and fill in your credentials:

```bash
cp .env.sample .env
```

3. Fill in your AWS credentials and other API keys in `.env` (see `.env.sample` for the full list of variables).

## Installation

```bash
cargo install --path .
```

## Usage

### Agent

Interactive AI agent session.

```bash
commander agent
```

| Command                                | Description                    |
| -------------------------------------- | ------------------------------ |
| `/new`                                 | Start a new session            |
| `/sessions`                            | Show recent sessions           |
| `/session <id>`                        | Switch to a session            |
| `/approve`                             | Approve pending tool execution |
| `/deny`                                | Deny pending tool execution    |
| `/tool-rules`                          | Show tool approval rules       |
| `/tool-rule <tool> <allow\|ask\|deny>` | Set tool approval rule         |
| `/attach <files...>`                   | Stage files to attach          |
| `/detach <files...>`                   | Remove files from staging      |
| `/attachments`                         | Show staged files              |
| `/help`                                | Show help                      |
| `/exit`                                | Quit                           |

Files can also be staged by dragging and dropping them onto the terminal window (bracketed paste).

### Research

Deep research on a given query. Saves a report to `outputs/research/`.

```bash
commander research
```

### Survey

Read and summarize an academic paper from a PDF file or URL. Saves a report to `outputs/survey/`.

```bash
commander survey <path-or-url> [--output <path>]
```

### Digest

Curate daily papers and tech news into a digest. Saves to `outputs/digest/`.

```bash
commander digest [--date <YYYY-MM-DD>] [--output <path>]
```

## Development

Run tests:

```bash
cargo test
```

Run lints:

```bash
cargo clippy
```
