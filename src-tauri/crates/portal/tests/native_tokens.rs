mod common;
use antigravity_portal::db;
pub use common::modules;
use common::{adapter::NativeUserTokens, original_tokens as native};
use serde_json::json;

#[test]
fn account_keys_use_original_creation_validation_rotation_and_usage() {
    let (_dir, portal) = common::portal();
    let user = portal
        .store
        .register("native@example.com", "fixture-password-long", db::now())
        .unwrap();
    let token = portal.ensure_native_token(&user).unwrap().unwrap();
    assert!(token.token.starts_with("sk-") && !token.token.starts_with("sk-portal-"));
    let original = native::get_token_by_value(&token.token).unwrap().unwrap();
    assert_eq!(original.id, token.id);
    assert_eq!(original.username, user.email);
    assert!(native::validate_token(&token.token, "127.0.0.1").unwrap().0);
    assert_eq!(
        portal.ensure_native_token(&user).unwrap().unwrap().id,
        token.id
    );

    // This is the same function called by the unchanged native monitor middleware.
    native::record_token_usage_and_ip(
        &token.id,
        "127.0.0.1",
        "fixture-model",
        12345,
        678,
        200,
        None,
    )
    .unwrap();
    let snapshot = portal.native_dashboard(&user.id).unwrap();
    let original = native::get_token_by_id(&token.id).unwrap().unwrap();
    assert_eq!(
        snapshot["key"]["total_tokens_used"],
        original.total_tokens_used
    );
    assert_eq!(snapshot["key"]["total_requests"], original.total_requests);
    assert_eq!(snapshot["key"]["usage"][0]["input_tokens"], 12345);
    assert_eq!(snapshot["key"]["usage"][0]["output_tokens"], 678);
    assert!(snapshot["key"].get("token").is_none());
    assert_eq!(snapshot["billing_enabled"], true);
    assert_eq!(snapshot["legacy"]["available"], 0);

    native::update_token(&token.id, None, None, Some(false), None, None, None).unwrap();
    assert!(!portal.ensure_native_token(&user).unwrap().unwrap().enabled);
    assert!(!native::validate_token(&token.token, "127.0.0.1").unwrap().0);
    portal.set_native_enabled(&user.id, true).unwrap();
    let replacement = portal.rotate_native_token(&user).unwrap();
    assert!(!native::validate_token(&token.token, "127.0.0.1").unwrap().0);
    assert!(native::validate_token(&replacement, "127.0.0.1").unwrap().0);
    let new_id = native::get_token_by_value(&replacement)
        .unwrap()
        .unwrap()
        .id;
    assert_ne!(new_id, token.id);
    // Old native usage remains available in the original administrator feature.
    assert_eq!(
        native::get_token_by_id(&token.id)
            .unwrap()
            .unwrap()
            .total_tokens_used,
        13023
    );
    portal.set_native_enabled(&user.id, false).unwrap();
    assert!(!native::validate_token(&replacement, "127.0.0.1").unwrap().0);
    native::delete_token(&new_id).unwrap();
    assert!(portal.ensure_native_token(&user).unwrap().is_none());
    assert!(portal.native_dashboard(&user.id).unwrap()["key"].is_null());
}

#[test]
fn existing_accounts_link_once_without_mutating_legacy_financial_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("portal.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(include_str!("../src/schema.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../src/model_settings.sql"))
        .unwrap();
    conn.execute_batch("INSERT INTO users(id,email,password_hash,created_at) VALUES('existing','old@example.com','fixture',123);
        INSERT INTO api_keys(id,user_id,digest,suffix,created_at) VALUES('key','existing','digest','suffix',123);
        INSERT INTO grants(id,user_id,source,kind,amount,remaining,reserved,created_at) VALUES('grant','existing','paid-order','purchase',1000000,900000,100,123);
        INSERT INTO ledger(user_id,event,kind,delta,reference,created_at) VALUES('existing','paid-order','purchase',1000000,'order',123);
        INSERT INTO requests(id,user_id,key_id,model,reserved,state,created_at) VALUES('request','existing','key','fixture',100,'review',123);").unwrap();
    let before: String = conn
        .query_row(
            "SELECT json_group_array(json_array(id,remaining,reserved)) FROM grants",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(conn);
    let store = db::Store::open(path.clone()).unwrap();
    let user = store.users().unwrap().remove(0);
    common::original_tokens::init_db().unwrap();
    let portal = antigravity_portal::Portal::new(
        store,
        common::portal().1.config.clone(),
        Some(std::sync::Arc::new(NativeUserTokens)),
    );
    let first = portal.ensure_native_token(&user).unwrap().unwrap();
    assert_eq!(
        portal.ensure_native_token(&user).unwrap().unwrap().id,
        first.id
    );
    let conn = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        conn.query_row("SELECT enabled FROM api_keys WHERE id='key'", [], |r| r
            .get::<_, bool>(0))
            .unwrap(),
        false
    );
    assert_eq!(
        conn.query_row(
            "SELECT json_group_array(json_array(id,remaining,reserved)) FROM grants",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        before
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM ledger", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        portal.native_dashboard(&user.id).unwrap()["legacy"]["paid_available"],
        json!(899900)
    );
    let reopened = db::Store::open(path).unwrap();
    assert_eq!(
        reopened.native_token_id(&user.id).unwrap().unwrap(),
        first.id
    );
}

#[test]
fn rotation_keeps_native_administrator_access_limits() {
    let (_dir, portal) = common::portal();
    let user = portal
        .store
        .register("limits@example.com", "fixture-password-long", db::now())
        .unwrap();
    let token = portal.ensure_native_token(&user).unwrap().unwrap();
    let conn = native::connect_db().unwrap();
    let expiry = db::now() - 60;
    conn.execute("UPDATE user_tokens SET expires_type='custom',expires_at=?1,max_ips=2,curfew_start='23:00',curfew_end='06:00',enabled=0 WHERE id=?2", rusqlite::params![expiry,token.id]).unwrap();
    let replacement = portal.rotate_native_token(&user).unwrap();
    let new = native::get_token_by_value(&replacement).unwrap().unwrap();
    assert_eq!(new.expires_at, Some(expiry));
    assert_eq!(new.max_ips, 2);
    assert_eq!(new.curfew_start.as_deref(), Some("23:00"));
    assert_eq!(new.curfew_end.as_deref(), Some("06:00"));
    assert!(!new.enabled);
    assert!(!native::validate_token(&replacement, "127.0.0.1").unwrap().0);
}
