//! Wikanvas's public API contract (D40, ARCHITECTURE §7): every request and response DTO of `/api/v1`, the error
//! envelope with `ErrorCode`, `Page<T>`, `Scope`, `API_VERSION`, the token format (`token`) and the CLI exit-code
//! table (`cli`). No I/O. The DTOs derive serde, ts-rs (`web/src/api/gen/*.ts`, `cargo test -p wikanvas-api
//! export_bindings`) and utoipa `ToSchema` (the OpenAPI spec, D41). Used by wikanvas-core (re-exported as
//! `wikanvas_core::api`), wikanvas-server and wikanvas-cli. This module list is frozen (PLAN W7 "Owns").

pub mod auth;
pub mod boards;
pub mod cli;
pub mod common;
pub mod docs;
pub mod error;
pub mod files;
pub mod page;
pub mod token;
pub mod tokens;
pub mod tree;

pub use auth::*;
pub use boards::*;
pub use common::{
    ChildOrder, ItemKind, Position, Role, Theme, Timestamp, TimestampError, UnknownValue, User,
};
pub use docs::*;
pub use error::*;
pub use files::*;
pub use page::{Page, PageQuery};
pub use tokens::*;
pub use tree::*;

/// The API version: the spec's `info.version`, `api_version` in `GET /api/v1/config`, and the `Wikanvas-Api-Version`
/// header on every `/api/v1` response. Frozen at "1.0" until the first release; after that a release bumps the minor
/// once when it adds anything (§7.1).
pub const API_VERSION: &str = "1.0";

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use utoipa::openapi::{RefOr, schema::Schema};
    use utoipa::{IntoParams, PartialSchema, ToSchema};

    use super::*;

    fn collect<T: ToSchema>() -> Value {
        let mut all: Vec<(String, RefOr<Schema>)> = vec![(T::name().into_owned(), T::schema())];
        T::schemas(&mut all);
        let map: serde_json::Map<String, Value> = all
            .into_iter()
            .map(|(name, s)| (name, serde_json::to_value(s).unwrap()))
            .collect();
        Value::Object(map)
    }

    #[test]
    fn openapi_schemas_build() {
        let s = collect::<ItemDetail>();
        for name in [
            "ItemDetail",
            "ItemSummary",
            "Collection",
            "Crumb",
            "BoardMeta",
            "Timestamp",
            "ItemKind",
        ] {
            assert!(s.get(name).is_some(), "{name} missing from {s:#}");
        }
        assert_eq!(s["Timestamp"]["type"], "string");
        assert_eq!(s["Timestamp"]["format"], "date-time");
        assert_eq!(s["ItemKind"]["enum"], json!(["doc", "board"]));
        let required = s["ItemDetail"]["required"].as_array().unwrap();
        assert!(required.contains(&json!("meta_version")) && !required.contains(&json!("board")));

        let page = serde_json::to_value(Page::<ItemSummary>::schema()).unwrap();
        assert!(page.to_string().contains("next_cursor"), "{page:#}");

        let errors = collect::<ErrorBody>();
        assert_eq!(
            errors["ErrorCode"]["enum"].as_array().unwrap().len(),
            ErrorCode::ALL.len()
        );

        let token = collect::<NewToken>();
        let required = token["NewToken"]["required"].as_array().unwrap();
        assert!(
            !required.contains(&json!("expires_in_days")) && required.contains(&json!("scopes"))
        );
    }

    #[test]
    fn query_params_build() {
        let params = SearchQuery::into_params(|| None);
        let names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        assert_eq!(
            names,
            [
                "q",
                "collection_id",
                "kind",
                "updated_after",
                "include_archived",
                "include_drafts",
                "sort",
                "cursor",
                "limit"
            ]
        );
        assert_eq!(PageQuery::into_params(|| None).len(), 2);
    }
}
