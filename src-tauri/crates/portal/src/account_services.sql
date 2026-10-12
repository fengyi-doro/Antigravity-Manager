-- Existing users keep access. Verification gates apply to new registrations only.
ALTER TABLE users ADD COLUMN email_verified INTEGER NOT NULL DEFAULT 1;
CREATE TABLE account_challenges (
 digest TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id),
 purpose TEXT NOT NULL CHECK(purpose IN ('verify','reset')), expires_at INTEGER NOT NULL
);
CREATE INDEX challenge_user ON account_challenges(user_id,purpose);
CREATE TABLE wallet_refunds (
 id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id),
 credit_reference TEXT NOT NULL REFERENCES wallet_credits(reference),
 amount_nano INTEGER NOT NULL CHECK(amount_nano>0),
 state TEXT NOT NULL CHECK(state IN ('requested','approved','paid','cancelled')),
 reason TEXT NOT NULL, payment_reference TEXT,
 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX refund_payment_reference ON wallet_refunds(payment_reference) WHERE payment_reference IS NOT NULL;
CREATE INDEX refund_user ON wallet_refunds(user_id,created_at DESC);
PRAGMA user_version=5;
