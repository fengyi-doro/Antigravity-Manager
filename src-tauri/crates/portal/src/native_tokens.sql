CREATE TABLE native_token_links (
    user_id TEXT PRIMARY KEY REFERENCES users(id),
    token_id TEXT NOT NULL UNIQUE
);
-- Existing portal credentials cannot authorize the new native-only API path.
UPDATE api_keys SET enabled=0;
PRAGMA user_version = 3;
