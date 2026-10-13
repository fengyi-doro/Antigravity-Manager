mod common;
use antigravity_portal::{billing::NANO_PER_YUAN, db, Error};
pub use common::modules;

#[test]
fn exact_prices_idempotency_rotation_failures_and_missing_usage() {
    let (_dir, p) = common::portal();
    let user = p
        .store
        .register("billing@example.com", "123456", db::now())
        .unwrap();
    let key = p.ensure_native_token(&user).unwrap().unwrap();
    assert!(matches!(
        p.store.begin_billing(&key.id, "model"),
        Err(Error::InsufficientBalance)
    ));
    p.store
        .credit_wallet(&user.id, 10_000, "initial-credit")
        .unwrap();
    p.store
        .credit_wallet(&user.id, 10_000, "initial-credit")
        .unwrap();
    assert!(matches!(
        p.store.credit_wallet(&user.id, 1, "initial-credit"),
        Err(Error::Conflict)
    ));
    let id = p.store.begin_billing(&key.id, "model").unwrap().unwrap();
    // 1m uncached input + 1m cache + 1m output = 0.60 + 0.06 + 3.00 yuan.
    p.store
        .settle_billing(&id, 200, Some((2_000_000, 1_000_000, 1_000_000)))
        .unwrap();
    p.store
        .settle_billing(&id, 200, Some((2_000_000, 1_000_000, 1_000_000)))
        .unwrap();
    let wallet = p.store.wallet_dashboard(&user.id).unwrap();
    assert_eq!(wallet["balance_nano"], 96_340_000_000_i64);
    assert_eq!(wallet["spent_nano"], 3_660_000_000_i64);
    let pending = p
        .store
        .begin_billing(&key.id, "before-rotation")
        .unwrap()
        .unwrap();
    let replacement = p.rotate_native_token(&user).unwrap();
    let next = common::original_tokens::get_token_by_value(&replacement)
        .unwrap()
        .unwrap();
    let already_authenticated = p
        .store
        .begin_billing(&key.id, "rotation-race")
        .unwrap()
        .unwrap();
    p.store
        .settle_billing(&already_authenticated, 400, None)
        .unwrap();
    p.store
        .settle_billing(&pending, 200, Some((1, 0, 1)))
        .unwrap();
    let failed = p.store.begin_billing(&next.id, "failed").unwrap().unwrap();
    p.store
        .settle_billing(&failed, 500, Some((1_000_000, 0, 1_000_000)))
        .unwrap();
    assert_eq!(
        p.store.wallet_dashboard(&user.id).unwrap()["spent_nano"],
        3_660_003_600_i64
    );
    let missing = p.store.begin_billing(&next.id, "missing").unwrap().unwrap();
    p.store.settle_billing(&missing, 200, None).unwrap();
    assert!(matches!(
        p.store.begin_billing(&next.id, "blocked"),
        Err(Error::Unavailable(_))
    ));
    p.store
        .settle_billing(&missing, 200, Some((0, 0, 0)))
        .unwrap();
    let crash = p.store.begin_billing(&next.id, "crash").unwrap().unwrap();
    p.store.recover_billing().unwrap();
    assert_eq!(
        p.store.wallet_dashboard(&user.id).unwrap()["usage"][0]["state"],
        "review"
    );
    p.store.settle_billing(&crash, 502, None).unwrap();
    assert!(p.store.begin_billing(&next.id, "recovered").is_ok());
    assert_eq!(NANO_PER_YUAN, 1_000_000_000);
}

#[test]
fn concurrent_settlements_preserve_all_charges_and_zero_balance_blocks() {
    let (_dir, p) = common::portal();
    let user = p
        .store
        .register("parallel@example.com", "123456", db::now())
        .unwrap();
    let key = p.ensure_native_token(&user).unwrap().unwrap();
    p.store.credit_wallet(&user.id, 300, "credit").unwrap();
    let ids: Vec<_> = (0..10)
        .map(|_| p.store.begin_billing(&key.id, "model").unwrap().unwrap())
        .collect();
    let threads: Vec<_> = ids
        .into_iter()
        .map(|id| {
            let store = p.store.clone();
            std::thread::spawn(move || {
                store
                    .settle_billing(&id, 200, Some((0, 0, 100_000)))
                    .unwrap()
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(
        p.store.wallet_dashboard(&user.id).unwrap()["balance_nano"],
        0
    );
    assert!(matches!(
        p.store.begin_billing(&key.id, "blocked"),
        Err(Error::InsufficientBalance)
    ));
}

#[test]
fn interrupted_streams_release_charges_and_recovery_survives_reopen() {
    let (dir, p) = common::portal();
    let user = p
        .store
        .register("interrupt@example.com", "123456", db::now())
        .unwrap();
    let key = p.ensure_native_token(&user).unwrap().unwrap();
    p.store
        .credit_wallet(&user.id, 100, "stream-credit")
        .unwrap();
    for status in [499, 502] {
        let id = p.store.begin_billing(&key.id, "stream").unwrap().unwrap();
        p.store
            .settle_billing(&id, status, Some((1000, 500, 200)))
            .unwrap();
        // A later duplicate success cannot change an already released request.
        p.store
            .settle_billing(&id, 200, Some((1000, 500, 200)))
            .unwrap();
    }
    assert_eq!(
        p.store.wallet_dashboard(&user.id).unwrap()["balance_nano"],
        NANO_PER_YUAN
    );
    let id = p.store.begin_billing(&key.id, "restart").unwrap().unwrap();
    let reopened = db::Store::open(dir.path().join("portal.db")).unwrap();
    reopened.recover_billing().unwrap();
    assert_eq!(
        reopened.wallet_dashboard(&user.id).unwrap()["review_count"],
        1
    );
    reopened
        .settle_billing(&id, 200, Some((1000, 800, 50)))
        .unwrap();
    let wallet = reopened.wallet_dashboard(&user.id).unwrap();
    let row = &wallet["usage"][0];
    assert_eq!(row["uncached_tokens"], 200);
    assert_eq!(row["breakdown"]["input_nano"], 120000);
    assert_eq!(row["breakdown"]["cached_nano"], 48000);
    assert_eq!(row["breakdown"]["output_nano"], 150000);
    assert_eq!(row["charged_nano"], 318000);
    assert!(row["settled_at"].is_i64());
    assert_eq!(wallet["review_count"], 0);
    assert!(matches!(
        reopened.settle_billing(&id, 100, None),
        Err(Error::Invalid(_))
    ));
}
