use antigravity_portal::{
    db::{self, Store},
    payment::{self, Payment},
    Error,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Barrier},
};

const TIME: i64 = 1_728_460_800;
fn setup() -> (tempfile::TempDir, Store, String) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("portal.db")).unwrap();
    let user = store
        .register("tester@example.com", "correct-horse-battery", TIME)
        .unwrap();
    (dir, store, user.id)
}
fn purchase(store: &Store, user: &str, time: i64, id: &str) {
    let order = store.create_order(user, 100, "wxpay", id, time).unwrap();
    store
        .credit_order(&order.id, id, 100, "wxpay", time)
        .unwrap();
}
#[test]
fn signup_once_and_expires_at_beijing_midnight() {
    let (_dir, store, user) = setup();
    let expiry = db::midnight_after(TIME);
    assert_eq!((expiry + 28_800) % 86_400, 0);
    assert!(matches!(
        store.register("TESTER@example.com", "another-password", TIME),
        Err(Error::Conflict)
    ));
    assert_eq!(
        store.dashboard(&user, TIME).unwrap()["gift_available"],
        5_000_000
    );
    assert_eq!(
        store.dashboard(&user, expiry - 1).unwrap()["available"],
        5_000_000
    );
    assert_eq!(store.dashboard(&user, expiry).unwrap()["available"], 0);
    assert_eq!(store.dashboard(&user, TIME).unwrap()["available"], 0);
}
#[test]
fn password_sessions_and_revocation() {
    let (_dir, store, user) = setup();
    assert!(store.login("tester@example.com", "wrong-password").is_err());
    assert_eq!(
        store
            .login("TESTER@example.com", "correct-horse-battery")
            .unwrap()
            .id,
        user
    );
    let (session, _) = store.create_session(&user, TIME).unwrap();
    assert!(store.session(&session, TIME + 7 * 86_400).is_err());
    let first = store.rotate_key(&user, TIME).unwrap();
    let second = store.rotate_key(&user, TIME + 1).unwrap();
    assert!(store.key_identity(&first).is_err());
    assert!(store.key_identity(&second).is_ok());
    store.set_enabled(&user, false).unwrap();
    assert!(store.key_identity(&second).is_err());
    assert!(store.session(&session, TIME).is_err());
}
#[test]
fn reserve_prefers_gift_and_settlement_is_idempotent() {
    let (_dir, store, user) = setup();
    purchase(&store, &user, TIME, "purchase-00000001");
    let key = store.rotate_key(&user, TIME).unwrap();
    let identity = store.key_identity(&key).unwrap();
    let request = store.reserve(&identity, "text", 1000, TIME).unwrap();
    assert_eq!(
        store.dashboard(&user, TIME).unwrap()["gift_available"],
        4_999_000
    );
    store.settle(&request, 100, 200, TIME + 1).unwrap();
    store.settle(&request, 100, 200, TIME + 1).unwrap();
    let dash = store.dashboard(&user, TIME).unwrap();
    assert_eq!(dash["gift_available"], 4_999_700);
    assert_eq!(dash["paid_available"], 1_000_000);
    assert_eq!(dash["reserved"], 0);
}
#[test]
fn refunding_after_midnight_does_not_revive_gift() {
    let (_dir, store, user) = setup();
    let expiry = db::midnight_after(TIME);
    let key = store.rotate_key(&user, TIME).unwrap();
    let request = store
        .reserve(&store.key_identity(&key).unwrap(), "text", 500, expiry - 1)
        .unwrap();
    store.settle(&request, 10, 10, expiry + 2).unwrap();
    let dash = store.dashboard(&user, expiry + 2).unwrap();
    assert_eq!(dash["available"], 0);
    assert_eq!(dash["reserved"], 0);
}
#[test]
fn concurrent_requests_cannot_spend_same_balance() {
    let (_dir, store, user) = setup();
    let time = db::midnight_after(TIME);
    purchase(&store, &user, time, "purchase-00000002");
    let key = store.rotate_key(&user, time).unwrap();
    let identity = store.key_identity(&key).unwrap();
    let barrier = Arc::new(Barrier::new(16));
    let threads = (0..16)
        .map(|_| {
            let store = store.clone();
            let identity = identity.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.reserve(&identity, "text", 100_000, time)
            })
        })
        .collect::<Vec<_>>();
    let results = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 10);
    assert!(results
        .iter()
        .filter(|r| r.is_err())
        .all(|r| matches!(r, Err(Error::Quota))));
    assert_eq!(store.dashboard(&user, time).unwrap()["available"], 0);
}
#[test]
fn restart_retains_uncertain_reservations_until_reconciliation() {
    let (dir, store, user) = setup();
    let key = store.rotate_key(&user, TIME).unwrap();
    let request = store
        .reserve(&store.key_identity(&key).unwrap(), "text", 1200, TIME)
        .unwrap();
    drop(store);
    let store = Store::open(dir.path().join("portal.db")).unwrap();
    store.recover_pending().unwrap();
    assert_eq!(store.dashboard(&user, TIME).unwrap()["reserved"], 1200);
    assert_eq!(store.admin_users().unwrap()["review"][0]["id"], request);
    store.settle(&request, 0, 0, TIME + 1).unwrap();
    assert_eq!(
        store.dashboard(&user, TIME).unwrap()["available"],
        5_000_000
    );
}
#[test]
fn overrun_becomes_debt_and_topup_covers_it() {
    let (_dir, store, user) = setup();
    let key = store.rotate_key(&user, TIME).unwrap();
    let identity = store.key_identity(&key).unwrap();
    let request = store.reserve(&identity, "text", 1000, TIME).unwrap();
    store.settle(&request, 5_000_000, 50, TIME + 1).unwrap();
    assert_eq!(store.dashboard(&user, TIME).unwrap()["debt"], 50);
    assert!(matches!(
        store.reserve(&identity, "text", 1, TIME),
        Err(Error::Quota)
    ));
    purchase(&store, &user, TIME + 2, "purchase-00000003");
    assert_eq!(
        store.dashboard(&user, TIME + 2).unwrap()["available"],
        999_950
    );
    assert_eq!(store.dashboard(&user, TIME + 2).unwrap()["debt"], 0);
}
#[test]
fn payment_is_atomic_exactly_once_and_owned() {
    let (_dir, store, user) = setup();
    let order = store
        .create_order(&user, 100, "alipay", "order-idempotent-01", TIME)
        .unwrap();
    let repeated = store
        .create_order(&user, 100, "alipay", "order-idempotent-01", TIME)
        .unwrap();
    assert_eq!(order.id, repeated.id);
    assert!(store.order(&order.id, "someone-else").is_err());
    assert!(store
        .credit_order(&order.id, "trade-1", 99, "alipay", TIME)
        .is_err());
    assert!(store
        .credit_order(&order.id, "trade-1", 100, "wxpay", TIME)
        .is_err());
    let barrier = Arc::new(Barrier::new(8));
    let tasks = (0..8)
        .map(|_| {
            let store = store.clone();
            let order = order.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store
                    .credit_order(&order.id, "trade-1", 100, "alipay", TIME)
                    .unwrap();
            })
        })
        .collect::<Vec<_>>();
    for t in tasks {
        t.join().unwrap();
    }
    assert_eq!(
        store.dashboard(&user, TIME).unwrap()["paid_available"],
        1_000_000
    );
    let other = store
        .create_order(&user, 100, "alipay", "order-idempotent-02", TIME)
        .unwrap();
    assert!(store
        .credit_order(&other.id, "trade-1", 100, "alipay", TIME)
        .is_err());
    assert_eq!(store.order(&other.id, &user).unwrap().state, "pending");
}
#[test]
fn signed_callbacks_reject_forgery_and_amount_changes() {
    let (_dir, store, user) = setup();
    let order = store
        .create_order(&user, 100, "wxpay", "order-idempotent-03", TIME)
        .unwrap();
    let key = "test-merchant-secret-only";
    let payment = Payment::new(
        "https://payments.example.com/",
        "123".into(),
        key.into(),
        "https://portal.example.com".into(),
    )
    .unwrap();
    let mut fields = BTreeMap::from([
        ("pid".into(), "123".into()),
        ("out_trade_no".into(), order.id.clone()),
        ("trade_no".into(), "trade-abc".into()),
        ("type".into(), "wxpay".into()),
        ("money".into(), "1.00".into()),
        ("trade_status".into(), "TRADE_SUCCESS".into()),
        ("sign_type".into(), "MD5".into()),
    ]);
    fields.insert("sign".into(), payment::signature(&fields, key));
    let mut forged = fields.clone();
    forged.insert("money".into(), "0.01".into());
    assert!(payment.notify(&store, &forged, TIME).is_err());
    payment.notify(&store, &fields, TIME).unwrap();
    payment.notify(&store, &fields, TIME).unwrap();
    assert_eq!(
        store.dashboard(&user, TIME).unwrap()["paid_available"],
        1_000_000
    );
    for invalid in [
        "NaN",
        "-1",
        "1.001",
        "1e2",
        " 1.00",
        "1.",
        "9223372036854775807.00",
    ] {
        assert!(payment::cents(invalid).is_err());
    }
    assert_eq!(payment::cents("1.2").unwrap(), 120);
}
