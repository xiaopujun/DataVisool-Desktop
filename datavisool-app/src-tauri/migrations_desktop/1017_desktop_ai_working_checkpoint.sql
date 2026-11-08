-- Task 工作状态与恢复血缘独立于 Thread 展示历史。

CREATE TABLE ai_agent_working_checkpoint (
  run_id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL,
  turn_id TEXT NOT NULL,
  checkpoint_version INTEGER NOT NULL CHECK (checkpoint_version >= 1),
  state_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
);

CREATE UNIQUE INDEX uq_ai_agent_working_checkpoint_task
  ON ai_agent_working_checkpoint(task_id);

CREATE INDEX idx_ai_agent_working_checkpoint_turn
  ON ai_agent_working_checkpoint(turn_id, updated_at DESC);

CREATE TABLE ai_agent_run_resume (
  run_id TEXT PRIMARY KEY,
  previous_run_id TEXT NOT NULL,
  turn_id TEXT NOT NULL,
  created_at INTEGER NOT NULL CHECK (created_at >= 0)
);

CREATE UNIQUE INDEX uq_ai_agent_run_resume_previous
  ON ai_agent_run_resume(previous_run_id);

CREATE INDEX idx_ai_agent_run_resume_turn
  ON ai_agent_run_resume(turn_id, created_at);
