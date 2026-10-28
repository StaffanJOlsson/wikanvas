//! Cross-workstream flows (owner: W7)
//!
//! W0b: one flow over the minimal routes — an admin token mints a write token, which creates a collection; revoking
//! the parent revokes the child (D42).

use quire_api::{Collection, CreatedToken, ErrorCode, Scope};
use serde_json::json;

use crate::common::{TestApp, assert_error};

#[tokio::test]
async fn minted_token_creates_collection_and_dies_with_its_parent() {
    let app = TestApp::with_token(Scope::Admin).await;
    let res = app
        .server
        .post("/api/v1/tokens")
        .json(&json!({"name": "agent", "scopes": ["write"], "expires_in_days": 1}))
        .await;
    res.assert_status(axum::http::StatusCode::CREATED);
    let child: CreatedToken = res.json();
    let agent = app.token_client(&child.token);

    let res = agent
        .post("/api/v1/collections")
        .json(&json!({"name": "Notes", "color": "#336699"}))
        .await;
    res.assert_status(axum::http::StatusCode::CREATED);
    let collection: Collection = res.json();
    assert_eq!(collection.name, "Notes");
    assert_eq!(collection.color, "#336699");
    assert_eq!(collection.item_count, 0);

    app.server
        .delete("/api/v1/tokens/current")
        .await
        .assert_status_no_content();
    let res = agent.get("/api/v1/auth/me").await;
    assert_error(&res, 401, ErrorCode::InvalidToken);
}
