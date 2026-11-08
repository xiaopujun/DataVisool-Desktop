CREATE TABLE IF NOT EXISTS project (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  des TEXT,
  state TEXT,
  pid INTEGER,
  data_json TEXT,
  cover TEXT,
  deleted INTEGER NOT NULL DEFAULT 0,
  create_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  update_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP)
);

CREATE INDEX IF NOT EXISTS idx_project_deleted_update_time ON project(deleted, update_time);
CREATE INDEX IF NOT EXISTS idx_project_pid_deleted_update_time ON project(pid, deleted, update_time);

CREATE TRIGGER IF NOT EXISTS trg_project_update_time
AFTER UPDATE ON project
FOR EACH ROW
BEGIN
  UPDATE project SET update_time = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TABLE IF NOT EXISTS font (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT,
  url TEXT,
  deleted INTEGER NOT NULL DEFAULT 0,
  create_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  update_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  user_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_font_deleted_update_time ON font(deleted, update_time);

CREATE TRIGGER IF NOT EXISTS trg_font_update_time
AFTER UPDATE ON font
FOR EACH ROW
BEGIN
  UPDATE font SET update_time = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TABLE IF NOT EXISTS common_database (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT,
  type TEXT NOT NULL,
  ip TEXT NOT NULL,
  port TEXT NOT NULL,
  database_name TEXT NOT NULL,
  username TEXT NOT NULL,
  password TEXT NOT NULL,
  url TEXT,
  des TEXT,
  deleted INTEGER NOT NULL DEFAULT 0,
  update_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  create_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  user_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_common_database_deleted_update_time ON common_database(deleted, update_time);

CREATE TRIGGER IF NOT EXISTS trg_common_database_update_time
AFTER UPDATE ON common_database
FOR EACH ROW
BEGIN
  UPDATE common_database SET update_time = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TABLE IF NOT EXISTS common_api (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT,
  url TEXT NOT NULL,
  method TEXT NOT NULL,
  params TEXT,
  headers TEXT,
  deleted INTEGER NOT NULL DEFAULT 0,
  update_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  create_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  user_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_common_api_deleted_update_time ON common_api(deleted, update_time);

CREATE TRIGGER IF NOT EXISTS trg_common_api_update_time
AFTER UPDATE ON common_api
FOR EACH ROW
BEGIN
  UPDATE common_api SET update_time = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TABLE IF NOT EXISTS dynamic_script (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT,
  url TEXT NOT NULL,
  type TEXT NOT NULL,
  project_id TEXT,
  deleted INTEGER NOT NULL DEFAULT 0,
  update_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  create_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  user_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_dynamic_script_deleted_update_time ON dynamic_script(deleted, update_time);
CREATE INDEX IF NOT EXISTS idx_dynamic_script_project_id ON dynamic_script(project_id);

CREATE TRIGGER IF NOT EXISTS trg_dynamic_script_update_time
AFTER UPDATE ON dynamic_script
FOR EACH ROW
BEGIN
  UPDATE dynamic_script SET update_time = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TABLE IF NOT EXISTS video (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  url TEXT NOT NULL,
  size INTEGER,
  type TEXT,
  user_id TEXT,
  create_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  update_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  deleted INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_video_deleted_update_time ON video(deleted, update_time);

CREATE TRIGGER IF NOT EXISTS trg_video_update_time
AFTER UPDATE ON video
FOR EACH ROW
BEGIN
  UPDATE video SET update_time = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TABLE IF NOT EXISTS three_d_model (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  url TEXT NOT NULL,
  size INTEGER,
  type TEXT,
  user_id TEXT,
  is_compressed INTEGER NOT NULL DEFAULT 0,
  create_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  update_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  deleted INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_three_d_model_deleted_update_time ON three_d_model(deleted, update_time);

CREATE TRIGGER IF NOT EXISTS trg_three_d_model_update_time
AFTER UPDATE ON three_d_model
FOR EACH ROW
BEGIN
  UPDATE three_d_model SET update_time = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TABLE IF NOT EXISTS remote_component (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  component_name TEXT,
  url TEXT NOT NULL,
  user_id TEXT,
  create_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  update_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  deleted INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_remote_component_deleted_update_time ON remote_component(deleted, update_time);

CREATE TRIGGER IF NOT EXISTS trg_remote_component_update_time
AFTER UPDATE ON remote_component
FOR EACH ROW
BEGIN
  UPDATE remote_component SET update_time = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TABLE IF NOT EXISTS project_release (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT,
  state INTEGER NOT NULL DEFAULT 1,
  encrypt INTEGER NOT NULL DEFAULT 0,
  password TEXT,
  data_json TEXT,
  project_id INTEGER,
  expiration_date INTEGER,
  user_id TEXT,
  deleted INTEGER NOT NULL DEFAULT 0,
  create_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  update_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP)
);

CREATE INDEX IF NOT EXISTS idx_project_release_deleted_update_time ON project_release(deleted, update_time);
CREATE INDEX IF NOT EXISTS idx_project_release_project_id ON project_release(project_id);

CREATE TRIGGER IF NOT EXISTS trg_project_release_update_time
AFTER UPDATE ON project_release
FOR EACH ROW
BEGIN
  UPDATE project_release SET update_time = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TABLE IF NOT EXISTS image (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  url TEXT NOT NULL,
  name TEXT,
  hash TEXT,
  user_id TEXT,
  deleted INTEGER NOT NULL DEFAULT 0,
  create_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  update_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP)
);

CREATE INDEX IF NOT EXISTS idx_image_deleted_update_time ON image(deleted, update_time);
CREATE INDEX IF NOT EXISTS idx_image_hash ON image(hash);

CREATE TRIGGER IF NOT EXISTS trg_image_update_time
AFTER UPDATE ON image
FOR EACH ROW
BEGIN
  UPDATE image SET update_time = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TABLE IF NOT EXISTS kv_config (
  k TEXT PRIMARY KEY,
  v TEXT NOT NULL,
  create_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  update_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP)
);

CREATE TRIGGER IF NOT EXISTS trg_kv_config_update_time
AFTER UPDATE ON kv_config
FOR EACH ROW
BEGIN
  UPDATE kv_config SET update_time = CURRENT_TIMESTAMP WHERE k = NEW.k;
END;
