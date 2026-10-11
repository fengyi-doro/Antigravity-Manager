-- CNY is stored in nano-yuan: no floating point or per-request cent rounding.
CREATE TABLE wallets (
    user_id TEXT PRIMARY KEY REFERENCES users(id),
    balance_nano INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE wallet_credits (
    reference TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES wallets(user_id),
    amount_nano INTEGER NOT NULL CHECK(amount_nano > 0),
    created_at INTEGER NOT NULL
);
CREATE TABLE billing_requests (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES wallets(user_id),
    token_id TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER,
    cached_tokens INTEGER,
    output_tokens INTEGER,
    charged_nano INTEGER,
    status INTEGER,
    state TEXT NOT NULL CHECK(state IN ('pending','review','settled','released')),
    created_at INTEGER NOT NULL,
    settled_at INTEGER
);
CREATE INDEX billing_user_time ON billing_requests(user_id,created_at DESC);
-- Keep ownership across rotation so already authenticated/in-flight calls stay billable.
CREATE TABLE billing_token_owners (
    token_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id)
);
INSERT INTO billing_token_owners(token_id,user_id) SELECT token_id,user_id FROM native_token_links;
PRAGMA user_version = 4;
