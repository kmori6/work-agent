# Database

This document is a short memo for the current database layout in `commander`.

## Overview

- PostgreSQL database name: `agent`
- Admin migrations: [`db/migration/admin`](../db/migration/admin)
- Application migrations: [`db/migration/agent`](../db/migration/agent)

`admin` is for PostgreSQL-level setup such as creating the `agent` database.
`agent` is for tables and indexes inside the `agent` database.

## Tables

### `chat_sessions`

- One row per chat session
- Primary key: `id UUID`
- Timestamps:
  - `created_at`
  - `updated_at`

This table is the container for a conversation thread.

### `chat_messages`

- One row per message in a session
- Primary key: `id UUID`
- Foreign key: `session_id -> chat_sessions.id`
- Core fields:
  - `role`
  - `kind`
  - `text`
  - `payload`
  - `created_at`

This table stores the ordered message history for a session.
`kind` separates plain text from tool-related messages.
`payload` stores structured tool data as `JSONB` when needed.
