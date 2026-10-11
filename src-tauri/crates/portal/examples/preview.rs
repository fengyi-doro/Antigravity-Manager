//! Local UI/API preview only. No upstream proxy or administrator API is served.
//! PORTAL_PREVIEW_DATA must point at a separate development directory.
use antigravity_portal::{
    db::Store,
    http::{router_with, Config},
    Portal,
};
use std::{path::PathBuf, sync::Arc};
#[tokio::main]
async fn main() {
    let data = PathBuf::from(
        std::env::var("PORTAL_PREVIEW_DATA").expect("Set a separate PORTAL_PREVIEW_DATA directory"),
    );
    let dist = std::env::var("PORTAL_PREVIEW_DIST")
        .expect("Set PORTAL_PREVIEW_DIST to the built frontend");
    let portal = Arc::new(Portal::new(
        Store::open(data.join("portal.db")).expect("database"),
        Config::from_env().expect("config"),
        None,
    ));
    let app =
        router_with(portal).fallback_service(tower_http::services::ServeDir::new(&dist).fallback(
            tower_http::services::ServeFile::new(format!("{dist}/index.html")),
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:18046")
        .await
        .unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}
