-- 001_init: initial schema for ditdev_be_rust.
-- Idempotent (IF NOT EXISTS) so it can run safely against the existing production Neon DB.

CREATE TABLE IF NOT EXISTS admins (
  id         SERIAL PRIMARY KEY,
  username   VARCHAR NOT NULL UNIQUE,
  password   VARCHAR NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS projects (
  id          SERIAL PRIMARY KEY,
  title       VARCHAR NOT NULL,
  description TEXT NOT NULL,
  thumbnail   VARCHAR,
  tags        TEXT[],
  created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS project_links (
  id         SERIAL PRIMARY KEY,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  type       VARCHAR NOT NULL,
  url        VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS certificates (
  id             SERIAL PRIMARY KEY,
  title          VARCHAR NOT NULL,
  provider       VARCHAR NOT NULL,
  thumbnail      VARCHAR,
  issue_date     DATE,
  credential_url VARCHAR,
  pdf_file       VARCHAR NOT NULL,
  created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS stats (
  id         SERIAL PRIMARY KEY,
  key        VARCHAR NOT NULL UNIQUE,
  value      INTEGER,
  label      VARCHAR NOT NULL,
  start_date DATE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS xp_global (
  id         INTEGER PRIMARY KEY DEFAULT 1,
  bonus_xp   BIGINT NOT NULL DEFAULT 0,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CONSTRAINT xp_global_singleton CHECK (id = 1)
);

-- Seed the singleton XP row (idempotent).
INSERT INTO xp_global (id, bonus_xp)
VALUES (1, 0)
ON CONFLICT (id) DO NOTHING;
