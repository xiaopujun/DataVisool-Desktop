-- NULL marks records written before custom and generated URLs were distinguished.
ALTER TABLE common_database ADD COLUMN url_is_custom INTEGER;
