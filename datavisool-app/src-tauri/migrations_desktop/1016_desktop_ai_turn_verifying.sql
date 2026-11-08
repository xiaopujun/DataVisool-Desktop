-- FinalResponse 候选进入完成审计时，Turn/TaskRun 使用独立 verifying 状态。
-- SQLite 不能原地修改 CHECK 约束，因此保留全部数据重建两张状态表。

ALTER TABLE ai_agent_turn RENAME TO ai_agent_turn_before_verifying;

DROP INDEX uq_ai_agent_turn_order;
DROP INDEX uq_ai_agent_turn_request;
DROP INDEX uq_ai_agent_turn_scope;
DROP INDEX uq_ai_agent_turn_single_active;
DROP INDEX uq_ai_agent_turn_single_project_active;
DROP INDEX idx_ai_agent_turn_thread_page;

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
      'verifying',
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

INSERT INTO ai_agent_turn SELECT * FROM ai_agent_turn_before_verifying;
DROP TABLE ai_agent_turn_before_verifying;

CREATE UNIQUE INDEX uq_ai_agent_turn_order ON ai_agent_turn(thread_id, turn_sequence);
CREATE UNIQUE INDEX uq_ai_agent_turn_request ON ai_agent_turn(request_id);
CREATE UNIQUE INDEX uq_ai_agent_turn_scope ON ai_agent_turn(thread_id, turn_id);
CREATE UNIQUE INDEX uq_ai_agent_turn_single_active
  ON ai_agent_turn(thread_id)
  WHERE status IN ('created', 'running', 'verifying', 'waitingFrontendTool', 'waitingUserInput', 'waitingUserConfirmation');
CREATE UNIQUE INDEX uq_ai_agent_turn_single_project_active
  ON ai_agent_turn(project_id)
  WHERE status IN ('created', 'running', 'verifying', 'waitingFrontendTool', 'waitingUserInput', 'waitingUserConfirmation');
CREATE INDEX idx_ai_agent_turn_thread_page ON ai_agent_turn(thread_id, turn_sequence DESC);

ALTER TABLE ai_agent_task_run RENAME TO ai_agent_task_run_before_verifying;

DROP INDEX uq_ai_agent_task_run_run_id;
DROP INDEX uq_ai_agent_task_run_attempt;
DROP INDEX uq_ai_agent_task_run_scope;
DROP INDEX idx_ai_agent_task_run_turn_status;

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
      'verifying',
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

INSERT INTO ai_agent_task_run SELECT * FROM ai_agent_task_run_before_verifying;
DROP TABLE ai_agent_task_run_before_verifying;

CREATE UNIQUE INDEX uq_ai_agent_task_run_run_id ON ai_agent_task_run(run_id);
CREATE UNIQUE INDEX uq_ai_agent_task_run_attempt ON ai_agent_task_run(turn_id, attempt_no);
CREATE UNIQUE INDEX uq_ai_agent_task_run_scope ON ai_agent_task_run(turn_id, run_id);
CREATE INDEX idx_ai_agent_task_run_turn_status ON ai_agent_task_run(turn_id, status, updated_at DESC);
