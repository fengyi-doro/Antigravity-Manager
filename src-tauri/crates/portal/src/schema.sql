CREATE TABLE users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL COLLATE NOCASE UNIQUE,
    password_hash TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
    created_at INTEGER NOT NULL
);
CREATE TABLE sessions (
    digest TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    csrf TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);
CREATE INDEX sessions_expiry ON sessions(expires_at);
CREATE TABLE api_keys (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    digest TEXT NOT NULL UNIQUE,
    suffix TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
    created_at INTEGER NOT NULL
);
CREATE INDEX api_keys_user ON api_keys(user_id, enabled);
CREATE TABLE grants (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    source TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK(kind IN ('signup','purchase')),
    amount INTEGER NOT NULL CHECK(amount > 0),
    remaining INTEGER NOT NULL CHECK(remaining >= 0),
    reserved INTEGER NOT NULL DEFAULT 0 CHECK(reserved >= 0 AND reserved <= remaining),
    expires_at INTEGER,
    created_at INTEGER NOT NULL
);
CREATE INDEX grants_available ON grants(user_id, expires_at);
CREATE TABLE requests (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    key_id TEXT NOT NULL REFERENCES api_keys(id),
    model TEXT NOT NULL,
    reserved INTEGER NOT NULL,
    input_tokens INTEGER,
    output_tokens INTEGER,
    charged INTEGER,
    debt INTEGER NOT NULL DEFAULT 0 CHECK(debt >= 0),
    state TEXT NOT NULL CHECK(state IN ('pending','review','settled','released')),
    created_at INTEGER NOT NULL,
    settled_at INTEGER
);
CREATE INDEX requests_user_time ON requests(user_id, created_at DESC);
CREATE TABLE allocations (
    request_id TEXT NOT NULL REFERENCES requests(id),
    grant_id TEXT NOT NULL REFERENCES grants(id),
    amount INTEGER NOT NULL CHECK(amount > 0),
    PRIMARY KEY(request_id, grant_id)
);
CREATE TABLE ledger (
    id INTEGER PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    event TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    delta INTEGER NOT NULL,
    reference TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX ledger_user_time ON ledger(user_id, created_at DESC);
CREATE TABLE orders (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    idempotency_key TEXT NOT NULL,
    amount_cents INTEGER NOT NULL CHECK(amount_cents > 0),
    tokens INTEGER NOT NULL CHECK(tokens > 0),
    channel TEXT NOT NULL CHECK(channel IN ('alipay','wxpay')),
    state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','paid','closed')),
    trade_no TEXT UNIQUE,
    created_at INTEGER NOT NULL,
    paid_at INTEGER,
    UNIQUE(user_id, idempotency_key)
);
CREATE INDEX orders_user_time ON orders(user_id, created_at DESC);
CREATE TABLE payment_events (
    trade_no TEXT PRIMARY KEY,
    order_id TEXT NOT NULL REFERENCES orders(id),
    amount_cents INTEGER NOT NULL,
    received_at INTEGER NOT NULL
);
CREATE TABLE rate_limits (
    bucket TEXT PRIMARY KEY,
    count INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE TABLE audit (
    id INTEGER PRIMARY KEY,
    action TEXT NOT NULL,
    subject TEXT NOT NULL,
    details TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
PRAGMA user_version = 1;
