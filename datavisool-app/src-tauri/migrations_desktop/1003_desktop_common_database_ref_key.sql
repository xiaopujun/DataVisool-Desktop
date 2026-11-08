ALTER TABLE common_database ADD COLUMN ref_key TEXT;

UPDATE common_database
SET ref_key = lower(hex(randomblob(16)))
WHERE ref_key IS NULL OR trim(ref_key) = '';

CREATE UNIQUE INDEX IF NOT EXISTS idx_common_database_ref_key
ON common_database(ref_key)
WHERE ref_key IS NOT NULL AND ref_key != '';

CREATE TRIGGER IF NOT EXISTS trg_common_database_ref_key_after_insert
AFTER INSERT ON common_database
FOR EACH ROW
WHEN NEW.ref_key IS NULL OR trim(NEW.ref_key) = ''
BEGIN
  UPDATE common_database
  SET ref_key = lower(hex(randomblob(16)))
  WHERE id = NEW.id;
END;
