-- Agent 直接画布动作的持久化幂等日志。
CREATE TABLE IF NOT EXISTS ai_agent_canvas_operation (
  operation_id TEXT PRIMARY KEY,
  tool_call_id TEXT NOT NULL CHECK (length(trim(tool_call_id)) > 0),
  tool_name TEXT NOT NULL CHECK (length(trim(tool_name)) > 0),
  kind TEXT NOT NULL CHECK (kind IN ('createComponents', 'patchComponents')),
  status TEXT NOT NULL CHECK (status IN ('prepared', 'applying', 'applied', 'rejected', 'failed')),
  task_id TEXT NOT NULL CHECK (length(trim(task_id)) > 0),
  run_id TEXT NOT NULL CHECK (length(trim(run_id)) > 0),
  thread_id TEXT NOT NULL CHECK (length(trim(thread_id)) > 0),
  turn_id TEXT NOT NULL CHECK (length(trim(turn_id)) > 0),
  project_id TEXT NOT NULL CHECK (length(trim(project_id)) > 0),
  manager_id TEXT,
  request_fingerprint TEXT NOT NULL CHECK (length(trim(request_fingerprint)) > 0),
  receipt_json TEXT NOT NULL,
  prepared_at INTEGER NOT NULL CHECK (prepared_at >= 0),
  updated_at INTEGER NOT NULL CHECK (updated_at >= prepared_at),
  applied_at INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_ai_agent_canvas_operation_tool_call
  ON ai_agent_canvas_operation(run_id, tool_call_id);

CREATE INDEX IF NOT EXISTS idx_ai_agent_canvas_operation_run
  ON ai_agent_canvas_operation(run_id, prepared_at);

CREATE INDEX IF NOT EXISTS idx_ai_agent_canvas_operation_status
  ON ai_agent_canvas_operation(run_id, status, updated_at);

CREATE INDEX IF NOT EXISTS idx_ai_agent_canvas_operation_turn
  ON ai_agent_canvas_operation(thread_id, turn_id, updated_at);
