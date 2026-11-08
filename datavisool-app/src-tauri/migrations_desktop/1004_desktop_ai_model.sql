CREATE TABLE IF NOT EXISTS ai_model (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL DEFAULT '',
    provider TEXT NOT NULL DEFAULT '',
    model_name TEXT NOT NULL DEFAULT '',
    base_url TEXT,
    api_key TEXT,
    description TEXT,
    temperature REAL NOT NULL DEFAULT 0.7,
    max_tokens INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1,
    is_default INTEGER NOT NULL DEFAULT 0,
    deleted INTEGER NOT NULL DEFAULT 0,
    create_time TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    update_time TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_ai_model_deleted_update_time
    ON ai_model (deleted, is_default DESC, enabled DESC, update_time DESC);

CREATE TRIGGER IF NOT EXISTS trg_ai_model_update_time
AFTER UPDATE ON ai_model
FOR EACH ROW
WHEN NEW.update_time = OLD.update_time
BEGIN
    UPDATE ai_model
    SET update_time = CURRENT_TIMESTAMP
    WHERE id = OLD.id;
END;
