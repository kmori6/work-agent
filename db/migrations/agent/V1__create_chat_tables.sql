CREATE TABLE chat_sessions (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  status TEXT NOT NULL DEFAULT 'idle'
    CHECK (status IN ('idle', 'running', 'awaiting_approval')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE chat_messages (
  id UUID PRIMARY KEY DEFAULT uuidv7(),
  session_id UUID NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,

  type TEXT NOT NULL CHECK (
    type IN ('message', 'tool')
  ),

  role TEXT NOT NULL CHECK (
    role IN ('system', 'user', 'assistant')
  ),

  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

  CHECK (
    (type = 'message' AND role IN ('system', 'user', 'assistant'))
    OR
    (type = 'tool' AND role IN ('user', 'assistant'))
  )
);

CREATE TABLE chat_message_contents (
  id UUID PRIMARY KEY DEFAULT uuidv7(),
  message_id UUID NOT NULL REFERENCES chat_messages(id) ON DELETE CASCADE,
  content_index INT NOT NULL CHECK (content_index >= 0),

  type TEXT NOT NULL CHECK (
    type IN (
      'input_text',
      'output_text',
      'tool_call',
      'tool_call_output'
    )
  ),

-- input_text/output_text
  text TEXT,

-- tool_call/tool_call_output
-- OpenAI Responses API maps these to function_call/function_call_output.
  call_id TEXT,
  tool_name TEXT,
  arguments JSONB,
  output JSONB,
  result_status TEXT CHECK (result_status IN ('success', 'error')),

  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  
-- constraints
  CONSTRAINT uq_chat_message_contents_message_index
    UNIQUE (message_id, content_index),
  
  CONSTRAINT chk_chat_message_contents_required_fields CHECK (
    (
        type IN ('input_text', 'output_text')
        AND text IS NOT NULL
    )
    OR
    (
        type = 'tool_call'
        AND call_id IS NOT NULL
        AND tool_name IS NOT NULL
        AND arguments IS NOT NULL
    )
    OR
    (
        type = 'tool_call_output'
        AND call_id IS NOT NULL
        AND output IS NOT NULL
    )
  )
);

CREATE INDEX idx_chat_messages_session
  ON chat_messages(session_id, id);
