ALTER TABLE common_api ADD COLUMN ref_key TEXT;

UPDATE common_api
SET ref_key = lower(hex(randomblob(16)))
WHERE ref_key IS NULL OR trim(ref_key) = '';

CREATE UNIQUE INDEX IF NOT EXISTS idx_common_api_ref_key
ON common_api(ref_key)
WHERE ref_key IS NOT NULL AND ref_key != '';

CREATE TRIGGER IF NOT EXISTS trg_common_api_ref_key_after_insert
AFTER INSERT ON common_api
FOR EACH ROW
WHEN NEW.ref_key IS NULL OR trim(NEW.ref_key) = ''
BEGIN
  UPDATE common_api
  SET ref_key = lower(hex(randomblob(16)))
  WHERE id = NEW.id;
END;
