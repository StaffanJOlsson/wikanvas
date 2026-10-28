//! The "Getting started" seed (owner: W2).
//!
//! §9.6.7: the collection "Getting started" (colour `#3E63DD`, icon `rocket`) with "Welcome to Quire" (a document),
//! and under it "Writing in Quire" (a document) and "Example board" (a board). Every item is published on create, like
//! every item (D23). The content comes from the fixtures W4 and W6 write: `fixtures/ydoc/getting-started.bin` (the
//! welcome document, a Yjs v1 update) and `fixtures/boards/getting-started.inkboard` (the board envelope), both
//! embedded at build time. "Writing in Quire" starts empty until a fixture for it exists.

use quire_core::api::{Collection, ItemKind, NewCollection, NewItem};
use rusqlite::Transaction;

use crate::blobs::BlobStore;
use crate::error::Result;

/// The welcome document (Yjs v1 update).
pub const WELCOME_YDOC: &[u8] = include_bytes!("../../../fixtures/ydoc/getting-started.bin");
/// The example board (an Inkboard envelope).
pub const EXAMPLE_BOARD: &str = include_str!("../../../fixtures/boards/getting-started.inkboard");

/// The seeded collection's name.
pub const COLLECTION_NAME: &str = "Getting started";
/// Its colour (§9.6.7).
pub const COLLECTION_COLOR: &str = "#3E63DD";
/// Its lucide icon.
pub const COLLECTION_ICON: &str = "rocket";
/// The item titles, in tree order.
pub const WELCOME: &str = "Welcome to Quire";
pub const WRITING: &str = "Writing in Quire";
pub const BOARD: &str = "Example board";

/// Seeds "Getting started" for `owner_id` inside the setup transaction (`POST /setup`, `admin create-owner`), using
/// the collection and item create functions and the document and board initializers. Returns the collection.
pub fn getting_started(
    tx: &Transaction<'_>,
    blobs: &BlobStore,
    owner_id: &str,
    now: i64,
) -> Result<Collection> {
    let collection = crate::collections::create(
        tx,
        owner_id,
        &NewCollection {
            name: COLLECTION_NAME.to_owned(),
            icon: Some(COLLECTION_ICON.to_owned()),
            color: Some(COLLECTION_COLOR.to_owned()),
            ..Default::default()
        },
        now,
    )?;
    let item = |parent: Option<uuid::Uuid>, kind: ItemKind, title: &str| {
        crate::items::create(
            tx,
            owner_id,
            &NewItem {
                collection_id: collection.id,
                parent_id: parent,
                kind,
                title: Some(title.to_owned()),
                position: None,
            },
            false,
            now,
        )
    };
    let welcome = item(None, ItemKind::Doc, WELCOME)?;
    crate::docs::init_with(tx, &welcome.id.to_string(), WELCOME_YDOC, now)?;
    item(Some(welcome.id), ItemKind::Doc, WRITING)?;
    let board = item(Some(welcome.id), ItemKind::Board, BOARD)?;
    crate::boards::init_with(tx, blobs, &board.id.to_string(), EXAMPLE_BOARD, now)?;
    crate::collections::get(tx, &collection.id.to_string())
}
