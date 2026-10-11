CREATE TABLE portal_settings (
    name TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
PRAGMA user_version = 2;
