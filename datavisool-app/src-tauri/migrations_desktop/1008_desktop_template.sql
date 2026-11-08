CREATE TABLE IF NOT EXISTS template (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  description TEXT,
  cover TEXT,
  template_json TEXT NOT NULL,
  user_id TEXT,
  deleted INTEGER NOT NULL DEFAULT 0,
  create_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
  update_time TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP)
);

CREATE INDEX IF NOT EXISTS idx_template_deleted_update_time ON template(deleted, update_time);
CREATE INDEX IF NOT EXISTS idx_template_name ON template(name);

CREATE TRIGGER IF NOT EXISTS trg_template_update_time
AFTER UPDATE ON template
FOR EACH ROW
BEGIN
  UPDATE template SET update_time = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;
