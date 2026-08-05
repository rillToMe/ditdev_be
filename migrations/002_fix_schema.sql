-- 002_fix_schema: align fresh databases (created by 001) with the production schema.
-- The existing production tables were created externally with TIMESTAMP columns and a
-- VARCHAR issue_date. On production these ALTERs are no-ops (columns already match);
-- on a fresh DB they correct the types 001 created.

ALTER TABLE admins ALTER COLUMN created_at TYPE TIMESTAMP;
ALTER TABLE projects ALTER COLUMN created_at TYPE TIMESTAMP;
ALTER TABLE projects ALTER COLUMN updated_at TYPE TIMESTAMP;
ALTER TABLE certificates ALTER COLUMN created_at TYPE TIMESTAMP;
ALTER TABLE certificates ALTER COLUMN issue_date TYPE VARCHAR;
ALTER TABLE stats ALTER COLUMN created_at TYPE TIMESTAMP;
ALTER TABLE stats ALTER COLUMN updated_at TYPE TIMESTAMP;
