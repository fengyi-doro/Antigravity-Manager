//! Exercise the real original token database and the production adapter.
#![allow(dead_code)]
#[path = "../../../../src/modules/portal_tokens.rs"]
pub mod adapter;
#[path = "../../../../src/modules/user_token_db.rs"]
pub mod original_tokens;
pub mod modules {
    pub use super::original_tokens as user_token_db;
    pub mod account {
        pub fn get_data_dir() -> Result<std::path::PathBuf, String> {
            static DATA: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
            Ok(DATA
                .get_or_init(|| tempfile::tempdir().unwrap())
                .path()
                .to_owned())
        }
    }
}
use antigravity_portal::{db::Store, http::Config, Portal};
use std::sync::Arc;
pub fn portal() -> (tempfile::TempDir, Arc<Portal>) {
    let dir = tempfile::tempdir().unwrap();
    original_tokens::init_db().unwrap();
    let portal = Arc::new(Portal::new(
        Store::open(dir.path().join("portal.db")).unwrap(),
        Config {
            public_url: "https://portal.example.com".into(),
            api_base_url: "https://api.example.com/v1".into(),
            secure_cookie: true,
            registration_open: true,
            trust_proxy: false,
        },
        Some(Arc::new(adapter::NativeUserTokens)),
    ));
    (dir, portal)
}
