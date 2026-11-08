-- AI Fusion 历史记录后端存储
-- 三张核心表：历史主记录、历史消息、历史资产

CREATE TABLE IF NOT EXISTS ai_fusion_history (
  id TEXT PRIMARY KEY,
  project_id INTEGER NOT NULL,
  request_id TEXT NOT NULL,
  trace_id TEXT,
  scene TEXT NOT NULL,
  status TEXT NOT NULL,
  applied INTEGER,
  title TEXT NOT NULL,
  summary TEXT NOT NULL,
  reason TEXT,
  provider TEXT NOT NULL,
  model_name TEXT NOT NULL,
  base_url TEXT,
  usage_json TEXT,
  result_json TEXT,
  detail_json TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  deleted INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY(project_id) REFERENCES project(id)
);

CREATE INDEX IF NOT EXISTS idx_ai_fusion_history_project_time
  ON ai_fusion_history(project_id, deleted, created_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_ai_fusion_history_request
  ON ai_fusion_history(project_id, request_id)
  WHERE deleted = 0;

CREATE TABLE IF NOT EXISTS ai_fusion_history_message (
  id TEXT PRIMARY KEY,
  history_id TEXT NOT NULL,
  project_id INTEGER NOT NULL,
  role TEXT NOT NULL,
  content TEXT NOT NULL,
  content_type TEXT NOT NULL,
  order_index INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  deleted INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY(history_id) REFERENCES ai_fusion_history(id),
  FOREIGN KEY(project_id) REFERENCES project(id)
);

CREATE INDEX IF NOT EXISTS idx_ai_fusion_history_message_history_order
  ON ai_fusion_history_message(history_id, deleted, order_index);

CREATE TABLE IF NOT EXISTS ai_fusion_history_asset (
  id TEXT PRIMARY KEY,
  project_id INTEGER NOT NULL,
  history_id TEXT,
  request_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  mime_type TEXT NOT NULL,
  original_name TEXT NOT NULL,
  relative_path TEXT NOT NULL,
  thumb_relative_path TEXT,
  byte_size INTEGER NOT NULL,
  width INTEGER NOT NULL,
  height INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  deleted_at INTEGER,
  deleted INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY(project_id) REFERENCES project(id),
  FOREIGN KEY(history_id) REFERENCES ai_fusion_history(id)
);

CREATE INDEX IF NOT EXISTS idx_ai_fusion_history_asset_project_request
  ON ai_fusion_history_asset(project_id, request_id, deleted);

CREATE INDEX IF NOT EXISTS idx_ai_fusion_history_asset_history
  ON ai_fusion_history_asset(history_id, deleted);

CREATE INDEX IF NOT EXISTS idx_ai_fusion_history_asset_project_hash
  ON ai_fusion_history_asset(project_id, sha256, deleted);