CREATE TABLE tool_call_approvals (
  id UUID PRIMARY KEY DEFAULT uuidv7(),
  session_id UUID NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
  tool_call_id TEXT NOT NULL,
  tool_name TEXT NOT NULL,
  arguments JSONB NOT NULL,
  decision TEXT NOT NULL CHECK (decision IN ('approved', 'denied')),
  decided_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_tool_call_approvals_session_decided
  ON tool_call_approvals(session_id, decided_at DESC, id DESC);

CREATE INDEX idx_tool_call_approvals_tool_decided
  ON tool_call_approvals(tool_name, decided_at DESC, id DESC);
