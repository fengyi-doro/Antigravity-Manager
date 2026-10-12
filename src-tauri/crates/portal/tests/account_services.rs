mod common;
use antigravity_portal::{db, Error};
pub use common::modules;

#[test]
fn mail_challenges_are_bound_expiring_single_use_and_reset_invalidates_sessions() {
    let (_dir, p) = common::portal();
    let now = db::now();
    let a = p
        .store
        .register("a@example.invalid", "old-password", now)
        .unwrap();
    let b = p
        .store
        .register("b@example.invalid", "old-password", now)
        .unwrap();
    p.store.require_verification(&a.id).unwrap();
    // Enabling mandatory verification also gates a previously issued native key.
    let native = p.ensure_native_token(&a).unwrap().unwrap();
    p.store
        .credit_wallet(&a.id, 100, "verification-credit")
        .unwrap();
    assert!(p
        .store
        .begin_billing_with_policy(&native.id, "model", true)
        .is_err());
    let (_, old) = p
        .store
        .issue_challenge(&a.email, "verify", now)
        .unwrap()
        .unwrap();
    let (_, code) = p
        .store
        .issue_challenge(&a.email, "verify", now)
        .unwrap()
        .unwrap();
    assert!(p
        .store
        .consume_challenge(&old, "verify", Some(&a.id), None, now)
        .is_err());
    assert!(p
        .store
        .consume_challenge(&code, "verify", Some(&b.id), None, now)
        .is_err());
    assert!(p
        .store
        .consume_challenge(&code, "reset", None, Some("new-password"), now)
        .is_err());
    p.store
        .consume_challenge(&code, "verify", Some(&a.id), None, now)
        .unwrap();
    assert!(p.store.email_verified(&a.id).unwrap());
    let billing = p
        .store
        .begin_billing_with_policy(&native.id, "model", true)
        .unwrap()
        .unwrap();
    p.store.settle_billing(&billing, 502, None).unwrap();
    assert!(p
        .store
        .consume_challenge(&code, "verify", Some(&a.id), None, now)
        .is_err());
    let (session, _) = p.store.create_session(&a.id, now).unwrap();
    let (_, expired) = p
        .store
        .issue_challenge(&a.email, "reset", now - 1201)
        .unwrap()
        .unwrap();
    assert!(p
        .store
        .consume_challenge(&expired, "reset", None, Some("new-password"), now)
        .is_err());
    let (_, reset) = p
        .store
        .issue_challenge(&a.email, "reset", now)
        .unwrap()
        .unwrap();
    p.store
        .consume_challenge(&reset, "reset", None, Some("new-password"), now)
        .unwrap();
    assert!(p.store.session(&session, now).is_err());
    assert!(p.store.login(&a.email, "old-password").is_err());
    assert!(p.store.login(&a.email, "new-password").is_ok());
    assert!(p
        .store
        .consume_challenge(&reset, "reset", None, Some("another-password"), now)
        .is_err());
    assert!(p
        .store
        .issue_challenge("missing@example.invalid", "reset", now)
        .unwrap()
        .is_none());
}

#[test]
fn refunds_reserve_once_require_balance_preserve_credit_limits_and_record_offline_payment() {
    let (_dir, p) = common::portal();
    let now = db::now();
    let a = p
        .store
        .register("refund@example.invalid", "password", now)
        .unwrap();
    let b = p
        .store
        .register("other@example.invalid", "password", now)
        .unwrap();
    p.store.credit_wallet(&a.id, 1000, "credit-a").unwrap();
    assert!(p
        .store
        .request_refund(&b.id, "credit-a", 100, "wrong owner", now)
        .is_err());
    let id = p
        .store
        .request_refund(&a.id, "credit-a", 600, "unused", now)
        .unwrap();
    assert_eq!(
        p.store.wallet_dashboard(&a.id).unwrap()["balance_nano"],
        10_000_000_000i64
    );
    assert!(p
        .store
        .request_refund(&a.id, "credit-a", 401, "over limit", now)
        .is_err());
    let key = p.ensure_native_token(&a).unwrap().unwrap();
    let request = p.store.begin_billing(&key.id, "model").unwrap().unwrap();
    assert!(p
        .store
        .transition_refund(&id, "approve", None, now)
        .is_err());
    p.store.settle_billing(&request, 502, None).unwrap();
    let threads = (0..4)
        .map(|_| {
            let store = p.store.clone();
            let id = id.clone();
            std::thread::spawn(move || store.transition_refund(&id, "approve", None, now))
        })
        .collect::<Vec<_>>();
    for t in threads {
        t.join().unwrap().unwrap();
    }
    assert_eq!(
        p.store.wallet_dashboard(&a.id).unwrap()["balance_nano"],
        4_000_000_000i64
    );
    assert!(p.store.transition_refund(&id, "pay", None, now).is_err());
    p.store
        .transition_refund(&id, "pay", Some("offline-001"), now)
        .unwrap();
    p.store
        .transition_refund(&id, "pay", Some("offline-001"), now)
        .unwrap();
    assert!(p
        .store
        .transition_refund(&id, "pay", Some("different"), now)
        .is_err());
    assert!(p.store.transition_refund(&id, "cancel", None, now).is_err());
    let other = p
        .store
        .request_refund(&a.id, "credit-a", 400, "unused", now)
        .unwrap();
    p.store
        .transition_refund(&other, "approve", None, now)
        .unwrap();
    assert!(matches!(
        p.store.begin_billing(&key.id, "model"),
        Err(Error::InsufficientBalance)
    ));
    assert!(p
        .store
        .transition_refund(&other, "pay", Some("offline-001"), now)
        .is_err());
    p.store
        .transition_refund(&other, "cancel", None, now)
        .unwrap();
    p.store
        .transition_refund(&other, "cancel", None, now)
        .unwrap();
    assert_eq!(
        p.store.wallet_dashboard(&a.id).unwrap()["balance_nano"],
        4_000_000_000i64
    );
    assert_eq!(
        p.store.wallet_history(&a.id).unwrap()["credits"][0]["refundable_nano"],
        4_000_000_000i64
    );
}

#[test]
fn v4_migration_preserves_existing_accounts_and_wallets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("portal.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    for sql in [
        include_str!("../src/schema.sql"),
        include_str!("../src/model_settings.sql"),
        include_str!("../src/native_tokens.sql"),
        include_str!("../src/billing.sql"),
    ] {
        conn.execute_batch(sql).unwrap();
    }
    conn.execute("INSERT INTO users(id,email,password_hash,created_at) VALUES('old','old@example.invalid','old-hash',1)",[]).unwrap();
    conn.execute("INSERT INTO wallets VALUES('old',123456)", [])
        .unwrap();
    drop(conn);
    let store = db::Store::open(path).unwrap();
    assert!(store.email_verified("old").unwrap());
    assert_eq!(
        store.wallet_dashboard("old").unwrap()["balance_nano"],
        123456
    );
}
