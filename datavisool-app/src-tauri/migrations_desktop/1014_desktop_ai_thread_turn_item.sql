-- Thread / Turn / Item 独立持久化。
--
-- 旧 Agent Runtime 不做数据迁移。该版本迁移只执行一次：先按子到父删除旧表及
-- 可能存在的开发版目标表，再创建正式结构。所有跨表关系均由 Repository 在事务
-- 中校验，不声明 SQLite 物理外键。

DROP TABLE IF EXISTS ai_agent_task_event;
DROP TABLE IF EXISTS ai_agent_turn_input;
DROP TABLE IF EXISTS ai_agent_conversation_message;
DROP TABLE IF EXISTS ai_agent_item;
DROP TABLE IF EXISTS ai_agent_task_run;
DROP TABLE IF EXISTS ai_agent_conversation;
DROP TABLE IF EXISTS ai_agent_turn;
DROP TABLE IF EXISTS ai_agent_attachment;
DROP TABLE IF EXISTS ai_fusion_attachment;
DROP TABLE IF EXISTS ai_agent_thread;
DROP TABLE IF EXISTS ai_agent_runtime_cleanup_marker;

CREATE TABLE ai_agent_thread (
  thread_id TEXT PRIMARY KEY,
  project_id INTEGER NOT NULL,
  title TEXT,
  title_source TEXT NOT NULL CHECK (title_source IN ('auto', 'user')),
  preview TEXT,
  status TEXT NOT NULL CHECK (status IN ('active', 'archived')),
  create_request_id TEXT NOT NULL CHECK (length(trim(create_request_id)) > 0),
  next_turn_sequence INTEGER NOT NULL DEFAULT 1 CHECK (next_turn_sequence >= 1),
  next_event_sequence INTEGER NOT NULL DEFAULT 1 CHECK (next_event_sequence >= 1),
  row_version INTEGER NOT NULL DEFAULT 1 CHECK (row_version >= 1),
  created_at INTEGER NOT NULL CHECK (created_at >= 0),
  updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
  last_activity_at INTEGER NOT NULL CHECK (last_activity_at >= 0),
  archived_at INTEGER,
  CHECK (
    (status = 'active' AND archived_at IS NULL)
    OR (status = 'archived' AND archived_at IS NOT NULL AND archived_at >= 0)
  )
);

CREATE INDEX idx_ai_agent_thread_project_activity
  ON ai_agent_thread(project_id, status, last_activity_at DESC, thread_id);

CREATE UNIQUE INDEX uq_ai_agent_thread_scope
  ON ai_agent_thread(project_id, thread_id);

CREATE UNIQUE INDEX uq_ai_agent_thread_create_request
  ON ai_agent_thread(project_id, create_request_id);

CREATE TABLE ai_agent_turn (
  turn_id TEXT PRIMARY KEY,
  project_id INTEGER NOT NULL,
  thread_id TEXT NOT NULL,
  turn_sequence INTEGER NOT NULL CHECK (turn_sequence >= 1),
  request_id TEXT NOT NULL CHECK (length(trim(request_id)) > 0),
  status TEXT NOT NULL CHECK (
    status IN (
      'created',
      'running',
      'waitingFrontendTool',
      'waitingUserInput',
      'waitingUserConfirmation',
      'completed',
      'cancelled',
      'failed',
      'interrupted'
    )
  ),
  request_json TEXT NOT NULL,
  input_summary TEXT,
  model_config_id TEXT NOT NULL,
  runtime_model_id TEXT NOT NULL,
  provider TEXT,
  entry_digest TEXT NOT NULL CHECK (length(trim(entry_digest)) > 0),
  usage_json TEXT,
  last_artifact_id TEXT,
  last_artifact_revision INTEGER,
  error_code TEXT,
  error_message TEXT,
  next_item_sequence INTEGER NOT NULL DEFAULT 1 CHECK (next_item_sequence >= 1),
  row_version INTEGER NOT NULL DEFAULT 1 CHECK (row_version >= 1),
  created_at INTEGER NOT NULL CHECK (created_at >= 0),
  started_at INTEGER,
  updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
  ended_at INTEGER,
  CHECK (started_at IS NULL OR started_at >= 0),
  CHECK (
    (status IN ('completed', 'cancelled', 'failed', 'interrupted') AND ended_at IS NOT NULL AND ended_at >= 0)
    OR (status NOT IN ('completed', 'cancelled', 'failed', 'interrupted') AND ended_at IS NULL)
  ),
  CHECK (
    (last_artifact_id IS NULL AND last_artifact_revision IS NULL)
    OR (last_artifact_id IS NOT NULL AND last_artifact_revision IS NOT NULL AND last_artifact_revision >= 0)
  ),
  CHECK (
    (error_code IS NULL AND error_message IS NULL)
    OR (error_code IS NOT NULL AND error_message IS NOT NULL)
  )
);

CREATE UNIQUE INDEX uq_ai_agent_turn_order
  ON ai_agent_turn(thread_id, turn_sequence);

CREATE UNIQUE INDEX uq_ai_agent_turn_request
  ON ai_agent_turn(request_id);

CREATE UNIQUE INDEX uq_ai_agent_turn_scope
  ON ai_agent_turn(thread_id, turn_id);

CREATE UNIQUE INDEX uq_ai_agent_turn_single_active
  ON ai_agent_turn(thread_id)
  WHERE status IN (
    'created',
    'running',
    'waitingFrontendTool',
    'waitingUserInput',
    'waitingUserConfirmation'
  );

CREATE UNIQUE INDEX uq_ai_agent_turn_single_project_active
  ON ai_agent_turn(project_id)
  WHERE status IN (
    'created',
    'running',
    'waitingFrontendTool',
    'waitingUserInput',
    'waitingUserConfirmation'
  );

CREATE INDEX idx_ai_agent_turn_thread_page
  ON ai_agent_turn(thread_id, turn_sequence DESC);

CREATE TABLE ai_agent_item (
  item_id TEXT PRIMARY KEY,
  turn_id TEXT NOT NULL,
  item_sequence INTEGER NOT NULL CHECK (item_sequence >= 1),
  source_run_id TEXT,
  logical_key TEXT,
  item_type TEXT NOT NULL CHECK (length(trim(item_type)) > 0),
  status TEXT NOT NULL CHECK (
    status IN ('created', 'inProgress', 'waiting', 'completed', 'failed', 'cancelled', 'declined')
  ),
  schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
  payload_json TEXT NOT NULL,
  last_event_sequence INTEGER NOT NULL DEFAULT 0 CHECK (last_event_sequence >= 0),
  row_version INTEGER NOT NULL DEFAULT 1 CHECK (row_version >= 1),
  created_at INTEGER NOT NULL CHECK (created_at >= 0),
  updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
  completed_at INTEGER,
  CHECK (
    (status IN ('completed', 'failed', 'cancelled', 'declined') AND completed_at IS NOT NULL AND completed_at >= 0)
    OR (status IN ('created', 'inProgress', 'waiting') AND completed_at IS NULL)
  )
);

CREATE UNIQUE INDEX uq_ai_agent_item_order
  ON ai_agent_item(turn_id, item_sequence);

CREATE UNIQUE INDEX uq_ai_agent_item_scope
  ON ai_agent_item(turn_id, item_id);

CREATE UNIQUE INDEX uq_ai_agent_item_logical_key
  ON ai_agent_item(turn_id, logical_key)
  WHERE logical_key IS NOT NULL;

CREATE INDEX idx_ai_agent_item_turn_page
  ON ai_agent_item(turn_id, item_sequence);

CREATE INDEX idx_ai_agent_item_source_run
  ON ai_agent_item(source_run_id);

CREATE TABLE ai_agent_task_run (
  task_id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  request_id TEXT NOT NULL,
  turn_id TEXT NOT NULL,
  attempt_no INTEGER NOT NULL CHECK (attempt_no >= 1),
  client_session_id TEXT NOT NULL,
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
  task_type TEXT NOT NULL CHECK (length(trim(task_type)) > 0),
  limits_json TEXT NOT NULL,
  pending_json TEXT,
  usage_json TEXT,
  last_artifact_id TEXT,
  last_artifact_revision INTEGER,
  error_code TEXT,
  error_message TEXT,
  rig_version TEXT,
  rig_snapshot_ref TEXT,
  project_snapshot_ref TEXT,
  next_event_sequence INTEGER NOT NULL DEFAULT 1 CHECK (next_event_sequence >= 1),
  created_at INTEGER NOT NULL CHECK (created_at >= 0),
  started_at INTEGER,
  updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
  ended_at INTEGER,
  CHECK (started_at IS NULL OR started_at >= 0),
  CHECK (
    (status IN ('completed', 'cancelled', 'failed') AND ended_at IS NOT NULL AND ended_at >= 0)
    OR (status NOT IN ('completed', 'cancelled', 'failed') AND ended_at IS NULL)
  ),
  CHECK (
    (last_artifact_id IS NULL AND last_artifact_revision IS NULL)
    OR (last_artifact_id IS NOT NULL AND last_artifact_revision IS NOT NULL AND last_artifact_revision >= 0)
  ),
  CHECK (
    (error_code IS NULL AND error_message IS NULL)
    OR (error_code IS NOT NULL AND error_message IS NOT NULL)
  )
);

CREATE UNIQUE INDEX uq_ai_agent_task_run_run_id
  ON ai_agent_task_run(run_id);

CREATE UNIQUE INDEX uq_ai_agent_task_run_attempt
  ON ai_agent_task_run(turn_id, attempt_no);

CREATE UNIQUE INDEX uq_ai_agent_task_run_scope
  ON ai_agent_task_run(turn_id, run_id);

CREATE INDEX idx_ai_agent_task_run_turn_status
  ON ai_agent_task_run(turn_id, status, updated_at DESC);

CREATE TABLE ai_agent_task_event (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  turn_id TEXT NOT NULL,
  item_id TEXT,
  run_id TEXT NOT NULL,
  sequence INTEGER NOT NULL CHECK (sequence >= 1),
  thread_sequence INTEGER NOT NULL CHECK (thread_sequence >= 1),
  kind TEXT NOT NULL CHECK (length(trim(kind)) > 0),
  payload_json TEXT NOT NULL,
  dedupe_key TEXT NOT NULL CHECK (length(trim(dedupe_key)) > 0),
  schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
  created_at INTEGER NOT NULL CHECK (created_at >= 0)
);

CREATE UNIQUE INDEX uq_ai_agent_event_run_sequence
  ON ai_agent_task_event(run_id, sequence);

CREATE UNIQUE INDEX uq_ai_agent_event_thread_sequence
  ON ai_agent_task_event(thread_id, thread_sequence);

CREATE UNIQUE INDEX uq_ai_agent_event_dedupe
  ON ai_agent_task_event(thread_id, dedupe_key);

CREATE INDEX idx_ai_agent_event_turn_sequence
  ON ai_agent_task_event(turn_id, thread_sequence);

CREATE INDEX idx_ai_agent_event_item_sequence
  ON ai_agent_task_event(item_id, thread_sequence)
  WHERE item_id IS NOT NULL;

CREATE TABLE ai_agent_turn_input (
  input_id TEXT PRIMARY KEY,
  turn_id TEXT NOT NULL,
  user_item_id TEXT NOT NULL,
  content_digest TEXT NOT NULL CHECK (length(trim(content_digest)) > 0),
  status TEXT NOT NULL CHECK (status IN ('accepted', 'consumed', 'abandoned')),
  row_version INTEGER NOT NULL DEFAULT 1 CHECK (row_version >= 1),
  created_at INTEGER NOT NULL CHECK (created_at >= 0),
  consumed_at INTEGER,
  abandoned_at INTEGER,
  CHECK (
    (status = 'accepted' AND consumed_at IS NULL AND abandoned_at IS NULL)
    OR (status = 'consumed' AND consumed_at IS NOT NULL AND consumed_at >= 0 AND abandoned_at IS NULL)
    OR (status = 'abandoned' AND consumed_at IS NULL AND abandoned_at IS NOT NULL AND abandoned_at >= 0)
  )
);

CREATE UNIQUE INDEX uq_ai_agent_turn_input_item
  ON ai_agent_turn_input(turn_id, user_item_id);

CREATE INDEX idx_ai_agent_turn_input_status
  ON ai_agent_turn_input(turn_id, status, created_at);

CREATE TABLE ai_agent_conversation (
  conversation_id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  project_id INTEGER NOT NULL,
  next_message_sequence INTEGER NOT NULL DEFAULT 1 CHECK (next_message_sequence >= 1),
  summary_json TEXT,
  summary_until_sequence INTEGER NOT NULL DEFAULT 0 CHECK (summary_until_sequence >= 0),
  summary_estimated_tokens INTEGER NOT NULL DEFAULT 0 CHECK (summary_estimated_tokens >= 0),
  summary_updated_at INTEGER,
  created_at INTEGER NOT NULL CHECK (created_at >= 0),
  updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
  CHECK (conversation_id = thread_id),
  CHECK (
    (summary_json IS NULL AND summary_updated_at IS NULL)
    OR (summary_json IS NOT NULL AND summary_updated_at IS NOT NULL AND summary_updated_at >= 0)
  )
);

CREATE UNIQUE INDEX uq_ai_agent_conversation_thread
  ON ai_agent_conversation(thread_id);

CREATE UNIQUE INDEX uq_ai_agent_conversation_scope
  ON ai_agent_conversation(project_id, thread_id);

CREATE INDEX idx_ai_agent_conversation_project_time
  ON ai_agent_conversation(project_id, updated_at DESC);

CREATE TABLE ai_agent_conversation_message (
  id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL,
  sequence INTEGER NOT NULL CHECK (sequence >= 1),
  turn_id TEXT,
  source_item_id TEXT,
  role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'tool')),
  content_json TEXT NOT NULL,
  estimated_tokens INTEGER NOT NULL CHECK (estimated_tokens >= 0),
  artifact_refs_json TEXT NOT NULL,
  created_at INTEGER NOT NULL CHECK (created_at >= 0)
);

CREATE UNIQUE INDEX uq_ai_agent_conversation_message_order
  ON ai_agent_conversation_message(conversation_id, sequence);

CREATE UNIQUE INDEX uq_ai_agent_conversation_message_source_item
  ON ai_agent_conversation_message(source_item_id)
  WHERE source_item_id IS NOT NULL;

CREATE INDEX idx_ai_agent_conversation_message_turn
  ON ai_agent_conversation_message(turn_id, sequence)
  WHERE turn_id IS NOT NULL;

CREATE TABLE ai_agent_attachment (
  attachment_id TEXT PRIMARY KEY,
  project_id INTEGER NOT NULL,
  upload_session_id TEXT NOT NULL,
  upload_request_id TEXT NOT NULL,
  claimed_thread_id TEXT,
  claimed_turn_id TEXT,
  claimed_at INTEGER,
  status TEXT NOT NULL CHECK (status IN ('temporary', 'claimed', 'expired', 'deleted')),
  mime_type TEXT NOT NULL,
  original_name TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  relative_path TEXT NOT NULL,
  byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
  width INTEGER CHECK (width IS NULL OR width > 0),
  height INTEGER CHECK (height IS NULL OR height > 0),
  expire_at INTEGER NOT NULL CHECK (expire_at >= 0),
  last_used_at INTEGER,
  created_at INTEGER NOT NULL CHECK (created_at >= 0),
  updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
  CHECK (last_used_at IS NULL OR last_used_at >= 0),
  CHECK (
    (claimed_thread_id IS NULL AND claimed_turn_id IS NULL AND claimed_at IS NULL)
    OR (claimed_thread_id IS NOT NULL AND claimed_turn_id IS NOT NULL AND claimed_at IS NOT NULL AND claimed_at >= 0)
  ),
  CHECK (
    (status = 'temporary' AND claimed_thread_id IS NULL)
    OR (status = 'claimed' AND claimed_thread_id IS NOT NULL)
    OR status IN ('expired', 'deleted')
  )
);

CREATE INDEX idx_ai_agent_attachment_project_session
  ON ai_agent_attachment(project_id, upload_session_id, status, created_at DESC);

CREATE INDEX idx_ai_agent_attachment_upload_request
  ON ai_agent_attachment(project_id, upload_session_id, upload_request_id);

CREATE INDEX idx_ai_agent_attachment_claim
  ON ai_agent_attachment(project_id, claimed_thread_id, claimed_turn_id, status)
  WHERE claimed_thread_id IS NOT NULL;

CREATE INDEX idx_ai_agent_attachment_expire
  ON ai_agent_attachment(status, expire_at);

CREATE TABLE ai_agent_runtime_cleanup_marker (
  migration_key TEXT PRIMARY KEY,
  schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
  status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'succeeded', 'failed')),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
  owner_token TEXT,
  lease_expires_at INTEGER,
  started_at INTEGER,
  finished_at INTEGER,
  error_message TEXT,
  created_at INTEGER NOT NULL CHECK (created_at >= 0),
  updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
  CHECK (owner_token IS NULL OR length(trim(owner_token)) > 0),
  CHECK (lease_expires_at IS NULL OR lease_expires_at >= 0),
  CHECK (
    (
      status = 'pending'
      AND owner_token IS NULL
      AND lease_expires_at IS NULL
      AND started_at IS NULL
      AND finished_at IS NULL
      AND error_message IS NULL
    )
    OR (
      status = 'running'
      AND owner_token IS NOT NULL
      AND lease_expires_at IS NOT NULL
      AND started_at IS NOT NULL
      AND lease_expires_at > started_at
      AND finished_at IS NULL
      AND error_message IS NULL
    )
    OR (
      status = 'succeeded'
      AND owner_token IS NULL
      AND lease_expires_at IS NULL
      AND started_at IS NOT NULL
      AND finished_at IS NOT NULL
      AND error_message IS NULL
    )
    OR (
      status = 'failed'
      AND owner_token IS NULL
      AND lease_expires_at IS NULL
      AND started_at IS NOT NULL
      AND finished_at IS NOT NULL
      AND error_message IS NOT NULL
    )
  )
);

INSERT INTO ai_agent_runtime_cleanup_marker (
  migration_key,
  schema_version,
  status,
  attempt_count,
  created_at,
  updated_at
) VALUES (
  '1014_desktop_ai_thread_turn_item',
  1,
  'pending',
  0,
  CAST(strftime('%s', 'now') AS INTEGER) * 1000,
  CAST(strftime('%s', 'now') AS INTEGER) * 1000
);
