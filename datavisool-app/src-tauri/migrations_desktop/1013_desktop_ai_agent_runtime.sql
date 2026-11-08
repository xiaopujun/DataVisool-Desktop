-- Rig Agent 项目稳定运行数据。
--
-- 这些表只保存项目自有协议，不保存 Rig 的内部类型。Rig 快照仅作为同版本恢复的
-- 可选引用；任务、等待项、Artifact 和会话消息必须可独立读取。

CREATE TABLE IF NOT EXISTS ai_agent_task_run (
  task_id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL UNIQUE,
  request_id TEXT NOT NULL UNIQUE,
  project_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  conversation_id TEXT NOT NULL,
  task_type TEXT NOT NULL,
  status TEXT NOT NULL CHECK (
    status IN (
      'created',
      'running',
      'waitingFrontendTool',
      'waitingUserInput',
      'waitingUserConfirmation',
      'completed',
      'cancelled',
      'failed'
    )
  ),
  limits_json TEXT NOT NULL,
  pending_json TEXT,
  last_artifact_id TEXT,
  last_artifact_revision INTEGER,
  error_code TEXT,
  error_message TEXT,
  project_snapshot_ref TEXT,
  rig_version TEXT,
  rig_snapshot_ref TEXT,
  next_event_sequence INTEGER NOT NULL DEFAULT 1 CHECK (next_event_sequence >= 1),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  ended_at INTEGER,
  CHECK (conversation_id = session_id),
  CHECK (
    (last_artifact_id IS NULL AND last_artifact_revision IS NULL)
    OR (last_artifact_id IS NOT NULL AND last_artifact_revision IS NOT NULL AND last_artifact_revision >= 0)
  ),
  CHECK (
    (error_code IS NULL AND error_message IS NULL)
    OR (error_code IS NOT NULL AND error_message IS NOT NULL)
  )
);

CREATE INDEX IF NOT EXISTS idx_ai_agent_task_run_project_status
  ON ai_agent_task_run(project_id, status, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_ai_agent_task_run_conversation_time
  ON ai_agent_task_run(conversation_id, created_at DESC);

CREATE TABLE IF NOT EXISTS ai_agent_task_event (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  sequence INTEGER NOT NULL CHECK (sequence >= 1),
  kind TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE(run_id, sequence),
  FOREIGN KEY(run_id) REFERENCES ai_agent_task_run(run_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_ai_agent_task_event_run_sequence
  ON ai_agent_task_event(run_id, sequence);

CREATE TABLE IF NOT EXISTS ai_agent_conversation (
  conversation_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  next_message_sequence INTEGER NOT NULL DEFAULT 1 CHECK (next_message_sequence >= 1),
  summary_json TEXT,
  summary_until_sequence INTEGER NOT NULL DEFAULT 0 CHECK (summary_until_sequence >= 0),
  summary_estimated_tokens INTEGER NOT NULL DEFAULT 0 CHECK (summary_estimated_tokens >= 0),
  summary_updated_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  CHECK (conversation_id = session_id),
  CHECK (
    (summary_json IS NULL AND summary_updated_at IS NULL)
    OR (summary_json IS NOT NULL AND summary_updated_at IS NOT NULL)
  )
);

CREATE INDEX IF NOT EXISTS idx_ai_agent_conversation_project_time
  ON ai_agent_conversation(project_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS ai_agent_conversation_message (
  id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL,
  sequence INTEGER NOT NULL CHECK (sequence >= 1),
  role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'tool')),
  content_json TEXT NOT NULL,
  estimated_tokens INTEGER NOT NULL CHECK (estimated_tokens >= 0),
  artifact_refs_json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE(conversation_id, sequence),
  FOREIGN KEY(conversation_id) REFERENCES ai_agent_conversation(conversation_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_ai_agent_conversation_message_order
  ON ai_agent_conversation_message(conversation_id, sequence);
