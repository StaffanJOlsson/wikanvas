# Wikanvas M2 parity ledger seed

The checklist of what today's React client does and the egui client must do too (`docs/ARCHITECTURE.md` §19, PLAN
"M2"). This file is the seed: F0 expands it into the ledger `docs/parity.md` (PLAN M2 F0 task 6), owned by C9 (PLAN M2
R18). Every row starts as `missing` there; C9 sets it to `ported` when the named test is in the tree and passes, and
`scripts/parity.sh` fails on any `missing` row in any section: the 202 items, the old tests (smoke steps, frame tests,
and one row per vitest file here, expanded to one row per `it`/`test` call site in F0), the new M2 behaviours, and the
manual checks. It also fails when a named Rust test is not in
`scripts/dev.sh cargo test --workspace --locked -- --list`.

Source: the client inventory of 2026-10-29 (202 behaviours, P-001-P-202, of `web/src` at main `d5d3c94`: 14,712 lines of
TS/TSX/JS in 132 files and 2,743 lines of CSS), brought up to date after the review with the M1.1 review's card changes
on main `9c6e60a` (§18.14's D93-D98, folded into P-141 to P-153). Paths in the "Behaviour" column's sources are in that
report; this ledger keeps the behaviour, the size, and adds the port workstream and the new test. Sizes: S up to a day,
M 1-4 days or a small decision, L 1-3 weeks or a component egui lacks (weights 0.5, 2.5 and 10 days).

**Workstreams** (PLAN M2): C1 data layer, C2 shell and sidebar, C3 pages and settings, C4 editor core, C5 editor
blocks and menus, C6 boards and cards, C7 server (read view, forms, `serve`), C8 desktop and web entries. "C3 (C7)" means
C3 owns the item and C7 supplies a part.

**Tests** (ARCHITECTURE §19.11):
- `L1 <crate>::<module>::tests::<name>`: a unit test in `mod tests` (`client` = `wikanvas-client`, `app` =
  `wikanvas-app`, `editor` = `wikanvas-editor`, `board` = `wikanvas-board`, `core` = `wikanvas-core`).
- `L2 ui::<file>::<name>`: a kittest scenario on `FakeBackend` in `crates/wikanvas-app/tests/ui/<file>.rs`;
  `L2 editor::<file>::<name>` in `crates/wikanvas-editor/tests/`; `L2 board::<file>::<name>` in `crates/wikanvas-board/tests/`.
- `L3 flows::<file>::<name>`: a kittest scenario against the real server in `crates/wikanvas/tests/flows/<file>.rs`.
- `L4 snapshot::<name>`: a lavapipe snapshot. `L5 B<n>`: the browser smoke (PLAN M2 C9). `L6`: the desktop smoke.
- `L7 api::<file>::<name>`: `crates/wikanvas-server/tests/api/<file>.rs`.
- `IB tests_widget::<name>`: Inkboard's native tests of patch 0008; `IE tests_embed::<name>`: those of 0005 (exist).

**Port column:** `same` (the behaviour as today), `changed: …` (a decided change, with its D-number), `gone: …`, or
`owner: …` (stays as the default until the owner answers, ARCHITECTURE §19.15). "(new)" marks the 42 items with no
direct test today.

## Totals

| Workstream | Items | S | M | L | Weighted days |
|---|---|---|---|---|---|
| C1 data layer | 17 | 12 | 4 | 1 | 26.0 |
| C2 shell and sidebar | 70 | 56 | 14 | 0 | 63.0 |
| C3 pages and settings | 45 | 35 | 10 | 0 | 42.5 |
| C4 editor core | 15 | 4 | 7 | 4 | 53.0 |
| C5 editor blocks and menus | 20 | 5 | 14 | 1 | 54.0 |
| C6 boards and cards | 28 | 16 | 11 | 1 | 45.5 |
| C7 server | 6 | 4 | 2 | 0 | 7.0 |
| C8 desktop and web entries | 1 | 1 | 0 | 0 | 0.5 |
| **Total** | **202** | **133** | **62** | **7** | **291.5** |

Two L items are weighed by their sized sub-items instead of 10 days each: P-084 (13.5 days) and P-094 (16.5 days), both
listed under group I. P-115 (L) counts 0 days, because M2 builds nothing for it (owner question 12). So C4 is
2.0 + 17.5 + P-080 10 + P-081 10 + P-084 13.5 + P-115 0 = 53.0, and C5 is 2.5 + 35.0 + P-094 16.5 = 54.0.

## A. Routing and deep links

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-001 | Route table (setup, login, home, search, starred, archive, trash, settings/:section, collection, item, doc, board); anything else, `/drafts` too, is "Page not found" | M | C2 | L1 app::route::tests::parse_print_every_route; L2 ui::routing::unknown_and_drafts_not_found | changed: a `Route` enum; web History (C8), desktop in-app history |
| P-002 | `/` redirects to `/home` | S | C2 | L1 app::route::tests::root_is_home; L5 B1 | same |
| P-003 | `:slugId` parse (last 10 base62, `-` before); `slugify` rules | S | C2 | L1 app::route::tests::slug_id_cases; L1 core::ids::tests (exist) | same; `wikanvas_core::ids::slugify` |
| P-004 | URL replaced with the canonical slug after load, no history entry | S | C2 | L2 ui::routing::canonical_slug_replaces_without_history; L5 B5 | same |
| P-005 | `/item/<uuid>` replaced by the item's page, Back skips it; unknown id is "Page not found" | S | C2 | L2 ui::routing::item_uuid_redirects_skipping_history; L3 flows::links::item_link_opens_and_back | same |
| P-006 | Item pages check the kind; `not_found` and other errors | S | C2 | L2 ui::routing::board_route_with_doc_id_not_found | same |
| P-007 | Page state in the URL (search filters, debounced 250 ms; Home tab); Back restores the query | S | C2 | L1 app::route::tests::search_query_round_trip; L2 ui::routing::search_back_restores_query | same (new) |
| P-008 | Open in a new tab (Mod+Enter, middle-click, Mod+click) | M | C2 (C8) | L2 ui::sidebar::mod_enter_requests_new_tab (the `Platform` fake records `OpenUrl::new_tab`); L5 B5 | changed: web opens a browser tab; desktop opens in place with history (owner: §19.15 q19) |
| P-009 | `?next=` accepted only as a same-origin path | S | C2 (C7) | L1 app::route::tests::next_param_rules; L7 api::pages::login_next_rejects_foreign | changed: also enforced by the server's `/login` form (D114) |
| P-010 | "Wikanvas was updated. Reload" banner, once | S | C2 | L2 ui::shell::update_banner_once_on_build_mismatch; L7 api::serve::app_assets_by_build | changed: the wasm's build id against `/config` on reconnect or an API version error; the assets live under `/app/{build}/` so Reload loads the new build (D141); never on the desktop |
| P-011 | Route error page with the message and Reload | S | C2 | L2 ui::shell::page_load_error_page; L5 B8 | changed: a page's load error shows this page; a Rust panic shows the "stopped" screen |

## B. Authentication and setup

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-012 | Auth gate: `/auth/me`; 401 → login with `next`; 409 → setup; `must_change_password` → forced password page; other error → Try again; server theme applies | S | C3 | L2 ui::auth::gate_states; L3 flows::auth::force_password_change; L3 flows::client::desktop_session_survives_password_change | changed: on the web a 401 navigates to the server's `/login?next=` (D114); on the desktop `LocalTransport` mints a new session on a 401 (D105) |
| P-013 | First-run setup form (code, workspace, name, email, password ≥ 12 bytes, repeat; field errors; → Home) | M | C7 (C8) | L7 api::pages::setup_form_creates_owner_and_redirects; L3 flows::setup::desktop_first_run; L5 B1 | changed: web = server HTML form; desktop = first-run dialog without code or password (§19.8.5) |
| P-014 | Sign-in page ("Keep me signed in" on; messages for invalid credentials, rate limit, network) | S | C7 | L7 api::pages::login_form_errors; L5 B2 | changed: server HTML form with `autocomplete` (D114) |
| P-015 | Lost session in place: "Sign in again" over the page, unsaved work kept | S | C3 | L2 ui::auth::lost_session_saves_outbox_then_navigates | changed: the web persists the outbox and the board snapshot, then navigates to `/login?next=` (D114, §19.4.5); the desktop never loses its session (D105) |
| P-016 | After any sign-in, drop `me` and refetch every query | S | C1 | L1 client::cache::tests::sign_in_clears_and_refetches | same |
| P-017 | Sign out: `POST /auth/logout`, close dialogs, clear toasts and cache, go to login | S | C3 | L3 flows::auth::sign_out_clears_cache; L5 B2 | changed: web through `POST /logout`; no sign-out on the desktop (one local owner) |
| P-018 | Session transport: same-origin cookie, never an `Authorization` header; uploads as octet-stream | S | C1 | L1 client::transport::tests::cookie_only_and_octet_stream; L3 flows::client::local_transport_passes_csrf | changed: three transports (D104, D105) |
| P-019 | Remember-me lifetime visible as staying signed in | S | C7 | L7 api::pages::login_remember_sets_max_age; L5 B2 | same, on the HTML form |

## C. Shell and layout

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-020 | Sidebar beside main; width 216-400 (260) and collapsed state persisted; drawer below 1024 px, closed by backdrop or route change; hidden sidebar inert | M | C2 | L2 ui::shell::sidebar_width_persisted; L2 ui::shell::drawer_below_1024; L1 app::prefs::tests::react_keys_read_once | changed: eframe storage, seeded once from the React client's `wikanvas.ui` and `wikanvas.theme` (§19.4.4); a hidden sidebar is absent from the AccessKit tree |
| P-021 | Resize handle: drag, double-click resets, ←/→ by 16 px, separator semantics | S | C2 | L2 ui::shell::resize_handle_keys_and_double_click | same (new) |
| P-022 | "Show sidebar" button while collapsed or narrow | S | C2 | L2 ui::shell::show_sidebar_when_collapsed | same |
| P-023 | Page header: toggle, `h1`, the page's buttons | S | C2 | L2 ui::shell::page_header_heading_and_toggle | same (new) |
| P-024 | A route change flushes pending saves; leaving prompts while anything is pending | S | C2 (C8) | L2 ui::shell::route_change_flushes_pending; L6 (window close flushes); L3 flows::client::minimised_window_still_saves; L5 B6 (the `beforeunload` prompt) | same on the web (a `beforeunload` handler, §19.9); changed on the desktop: it flushes on window close, Ctrl+Q and when the window is minimised |

## D. Sidebar tree

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-025 | Workspace header: logo or letter tile, names, menu (Settings, Theme, Shortcuts, About, Sign out) | S | C2 | L2 ui::sidebar::workspace_menu_entries | changed: no Sign out on the desktop |
| P-026 | Nav (Home, Search, Starred) and footer (Archive, Trash, Settings); no Drafts, Templates or Invite | S | C2 | L2 ui::sidebar::nav_has_no_drafts; L3 flows::items::star_and_no_drafts | same |
| P-027 | STARRED section only when something is starred, in star order, current highlighted | S | C2 | L2 ui::sidebar::starred_only_when_any | same |
| P-028 | Star reorder by drag, both neighbours sent, optimistic with rollback and toast | M | C2 | L2 ui::sidebar::star_reorder_sends_neighbours; L2 ui::sidebar::star_reorder_rolls_back | same |
| P-029 | Flat visible rows, carets, kind icons, indent 8 + 16 px per level capped at 10, row height 30 | M | C2 | L1 app::tree::visible_rows::tests (the 6 cases) | same |
| P-030 | Trees load only when expanded; `If-None-Match`, 304 keeps | S | C2 (C1) | L2 ui::sidebar::tree_loads_on_expand; L1 client::cache::tests::tree_etag_304_keeps | same |
| P-031 | Sibling order (manual, A-Z by title key, updated, created; ties; orphans and cycles as roots) | S | C2 | L1 app::tree::build::tests (the 7 cases, `fixtures/tree/title-order.json`) | same; `wikanvas_core::tree::title_key` |
| P-032 | Expanded rows persisted, at most 5,000, oldest dropped | S | C2 | L1 app::tree::expanded::tests::cap_5000_oldest_dropped | changed: an eframe storage key, seeded once from `wikanvas.sidebar.expanded` |
| P-033 | Current item highlighted, its ancestors expanded once per item | S | C2 | L2 ui::sidebar::current_item_expands_ancestors_once; L3 flows::items::nested_docs_and_reload | same |
| P-034 | Virtualisation above 300 visible rows | S | C2 | L2 ui::sidebar::ten_thousand_rows_lays_out_visible_only (a row count) | changed: always virtualised with `show_rows` (D132) |
| P-035 | Row hover actions: sort, "+", "⋯" menus; visible while a menu is open or the row has focus | M | C2 | L2 ui::sidebar::row_menu_entries; L2 ui::sidebar::actions_stay_while_menu_open | same |
| P-036 | Inline rename (menu or F2): Enter saves, Esc cancels, blur saves; unchanged sends nothing | S | C2 | L2 ui::sidebar::f2_rename_enter_esc_blur | same |
| P-037 | Rows as tree items with one roving tab stop | S | C2 | L2 ui::sidebar::one_tab_stop | changed: AccessKit tree roles |
| P-038 | "+ New collection…" at the end; load error with Retry | S | C2 | L2 ui::sidebar::new_collection_and_retry | same |
| P-039 | Auto-scroll near the edges while dragging | S | C2 | L2 ui::sidebar::drag_near_edge_scrolls | same (new) |
| P-040 | Item drop zones 25/50/25; "after" an expanded parent = before its first child; outdent left of the indent | M | C2 | L1 app::tree::hit_zones::tests (the 10 cases) | same |
| P-041 | Non-manual sibling groups: before and after become "into the parent" | S | C2 | L1 app::tree::drop::tests::non_manual (4) | same |
| P-042 | The dragged item and its descendants are no targets | S | C2 | L1 app::tree::drop::tests::invalid_targets (4) | same |
| P-043 | Drop on a collection row: end of its top level, the subtree moves along | S | C2 | L2 ui::sidebar::drop_on_collection_moves_subtree | same |
| P-044 | Drop indicators: line at the target depth, or the row boxed | S | C2 | L1 app::tree::hit_zones::tests::indicator; L4 snapshot::sidebar_drop_indicators | same |
| P-045 | Hovering "inside" a collapsed row for 700 ms expands it | S | C2 | L1 app::tree::auto_expand::tests (4, manual clock) | same |
| P-046 | Move body names both neighbours; "inside" and collection drops send no position; a no-op sends nothing | M | C2 | L1 app::tree::positions::tests (the 10 cases) | same |
| P-047 | Optimistic move with a display-only key; rollback with a toast on any error | M | C2 | L2 ui::sidebar::optimistic_move_rolls_back_on_409 | same |
| P-048 | "Moved to <parent> · Undo" toast (8 s); keyboard moves show none | S | C2 | L2 ui::sidebar::move_toast_undo | same |
| P-049 | Collection reorder by drag, optimistic with rollback | M | C2 | L2 ui::sidebar::collection_reorder (3 cases) | same |
| P-050 | Document rows are drag sources for boards, board rows for documents (D145); collection rows carry nothing | S | C2 | L2 ui::sidebar::doc_row_carries_item_payload; L2 ui::sidebar::board_row_carries_board_payload; L3 flows::board_embeds::sidebar_drag | changed: an egui drag payload, no native drag type (D132) |
| P-051 | Tree keys (↑/↓, Home/End, →/←, Enter, Mod+Enter, Shift+F10, Esc) | M | C2 | L2 ui::sidebar::keyboard_navigation (the cases of keyboard.test.tsx) | same |
| P-052 | Alt+Shift+arrows indent, outdent and reorder; focus stays on the row | S | C2 | L2 ui::sidebar::alt_shift_moves; L1 app::tree::positions::tests::keyboard | same |
| P-053 | Delete moves to Trash (confirmation with nested pages), focus to the next row | S | C2 | L2 ui::sidebar::delete_confirms_nested_and_focuses_next | same |
| P-054 | Type-ahead within 500 ms; one letter cycles; a dead prefix restarts | S | C2 | L1 app::tree::type_ahead::tests (3) | same |

## E. Collections

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-055 | Collection dialog: name ≤ 100, description, 12 colours, 48 icons or letter tile, preview; create expands and opens | M | C3 | L2 ui::collections::dialog_create_edit_preview | same |
| P-056 | Collection icon tinted, else the letter tile; unknown names fall back | M | C3 | L2 ui::collections::unknown_icon_falls_back; L4 snapshot::collection_icons | changed: lucide SVGs rasterised with resvg (D133) |
| P-057 | Collection page: header, New document, New board, ⋯, Sort, top-level items with "N nested", empty text | M | C3 | L2 ui::collections::page_lists_top_level_with_nested_count | same (new) |
| P-058 | Sort change `PATCH {child_order}`, optimistic everywhere cached | S | C3 | L2 ui::collections::sort_patch_optimistic | same (new) |
| P-059 | Delete: confirmation with the count, soft delete, optimistic, "Moved to Trash · Undo" | S | C3 | L2 ui::collections::delete_to_trash_with_undo | same (new) |

## F. Item actions

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-060 | Create a document or board (top, sibling, child), expand the parent, open it as "created here" | S | C3 | L2 ui::items::create_opens_created_here; L3 flows::items::nested_docs_and_reload | same |
| P-061 | An untitled empty item created here is discarded on leave (`discard_empty`) | S | C3 | L3 flows::items::untitled_empty_discarded_on_leave | same |
| P-062 | Title autosave 500 ms, pending, flushed on blur, route change, Mod+S; error with Retry | M | C3 | L2 ui::items::title_autosave_debounce_and_retry (manual clock) | same |
| P-063 | Duplicate, added to the tree, the copy opens | S | C3 | L3 flows::items::duplicate_opens_copy | same |
| P-064 | Star and unstar, optimistic | S | C3 | L2 ui::items::star_optimistic; L3 flows::items::star_and_no_drafts | same |
| P-065 | Archive with "Archived (with N nested) · Undo" | S | C3 | L2 ui::items::archive_toast_and_undo | same (new) |
| P-066 | Move to Trash with confirmation, optimistic, Undo, then open the parent | S | C3 | L3 flows::trash::trash_and_restore | same |
| P-067 | Move dialog: filter, everything but the item's subtree, Move goes last, disabled when unchanged | M | C3 | L2 ui::items::move_dialog_filters_and_excludes_subtree; L3 flows::items::move_dialog | same |
| P-068 | Record a view once per opened item, refresh Recently viewed | S | C3 | L3 flows::lists::recent_after_view | same |

## G. Top bar and item header

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-069 | Breadcrumbs (icon, ancestors, title; on boards an auto-sizing title input) | S | C3 | L2 ui::topbar::breadcrumbs_and_board_title_input | same |
| P-070 | Save status texts with Retry for error and offline | S | C3 | L1 client::pending::tests::save_status_states; L2 ui::topbar::status_node_values | changed: an AccessKit status node "Save status" |
| P-071 | "+ New doc" split button (click sibling, Alt-click child, menu), hidden below 1024 px | S | C3 | L2 ui::topbar::new_doc_click_alt_click_menu | same (new) |
| P-072 | Item ⋯ menu entries, disabled when trashed or archived | S | C3 | L2 ui::items::menu_entries_disabled_when_trashed | changed: adds "Open reading view" and "Copy reading link" (§19.7.6) |
| P-073 | Full width, applied at once, rolled back on failure | S | C3 | L2 ui::items::full_width_optimistic_rollback | same |
| P-074 | Item header: title input (read-only when trashed or archived, Enter to body, IME-safe), star, meta line refreshed every minute | S | C3 | L2 ui::items::header_enter_moves_to_body; L2 ui::items::meta_line_refreshes | same (IME through egui's `TextEdit`) |
| P-075 | Relative time strings | S | C3 | L1 app::time::tests (the cases of time.test.ts, on a `ManualClock` in a fixed time zone) | same; wall time and the local offset come from `Clock` (§19.4.2) |

## H. List pages

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-076 | Home: Recently viewed / updated tabs, 25 per page, Load more | S | C3 | L3 flows::lists::recent_after_view; L2 ui::lists::home_load_more | same |
| P-077 | Starred page with Unstar per row | S | C3 | L2 ui::lists::starred_unstar | same (new) |
| P-078 | Archive page with Unarchive (and "moved to top" toast) | S | C3 | L2 ui::lists::archive_unarchive_moved_to_top | same (new) |
| P-079 | List row: icon, title, path, meta, actions | S | C3 | L2 ui::lists::row_path_and_meta | same (new) |

## I. Documents and the editor

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-080 | Schema v1 as the document model (19 nodes, 7 marks) | L | C4 | L1 editor::model::tests::names_match_schema_v1 (against `fixtures/schema/v1.json`); L2 editor::corpus::reads_every_fixture | same (`wikanvas_core::schema` reused) |
| P-081 | Read and write exactly y-tiptap's Yjs XML; apply remote updates in place; open every fixture and server-written document | L | C4 | L2 editor::corpus::reads_every_fixture, ::open_close_writes_nothing, ::writes_tiptap_shape; `web/src/editor/serverWritten.test.ts` over `fixtures/ydoc/egui/` during the port | changed: direct yrs editing (D107) |
| P-082 | `blockId` on every block; never generated for server or remote changes | M | C4 | L1 editor::commands::tests::new_blocks_get_ids, ::paste_dedupes_ids, ::remote_blocks_untouched | same |
| P-083 | Undo and redo of local changes only | M | C4 | L2 editor::undo::ten_undos_restore, ::remote_never_undone; L3 flows::board_embeds::ten_undos | same (yrs `UndoManager`, D111) |
| P-084 | The editing core: caret, selection, typing, Enter/Backspace/Delete per node, list keys, gap cursor, clipboard, drag-select, scroll into view (weighed by sub-items: 13.5 days) | L | C4 | L2 editor::typing::*, editor::selection::*, editor::delete::*; L3 flows::items::nested_docs_and_reload, flows::docs::delete_body_then_retype | changed: Wikanvas's own widget |
| P-084.1 | Caret and motions (grapheme, word, row with goal x, block edges, document edges, page keys) | M | C4 | L1 editor::select::tests::motions | sub-item |
| P-084.2 | Typing and Enter per node | M | C4 | L2 editor::typing::enter_per_node | sub-item |
| P-084.3 | Backspace and Delete per node (join, lift, node selection) | M | C4 | L2 editor::delete::* | sub-item |
| P-084.4 | List keys (Tab, Shift+Tab, Enter in an empty item) | S | C4 | L2 editor::typing::list_keys | sub-item |
| P-084.5 | Gap cursor | M | C4 | L2 editor::selection::gap_cursor | sub-item |
| P-084.6 | Mouse: click, double and triple click, drag-select with auto-scroll | M | C4 | L2 editor::selection::mouse | sub-item |
| P-084.7 | Scroll into view after every move | S | C4 | L2 editor::selection::scroll_into_view | sub-item (the clipboard is P-086 and P-200) |
| P-085 | IME and composition, dead keys, CJK | M | C4 | L2 editor::ime::preedit_not_in_yrs, ::commit_is_one_undo_step, ::remote_update_during_composition; L5 B3 | same on desktops (new); changed on phones: eframe's text agent holds no text around the caret (eframe 0.37 `src/web/text_agent.rs:283`), so predictive text and autocorrection degrade; S0 G5 measures it, and phones open the read view by default (D130) |
| P-086 | HTML paste rules: callouts, board embeds, allow-listed images, item links from `data-item-id` | M | C4 | L1 editor::html_import::tests; L2 editor::clipboard::paste_corpus | same (html5ever and the allow-list, D112) |
| P-087 | Code block: 34 languages + Plain text, unknown kept, Mod+Alt+C, triple Enter exits | M | C5 | L2 editor::blocks::code_language_picker_and_triple_enter | changed: Inkboard's highlighter for 14 languages, others plain (D136) |
| P-088 | Highlight mark, `==x==`, Mod+Shift+H | S | C4 | L1 editor::input_rules::tests::highlight | same |
| P-089 | Link mark allow-list, autolink, link on paste, item links as `wikanvas://item/<uuid>` | M | C5 | L1 editor::links::tests::allow_list (the cases of link.test.ts) | same |
| P-090 | Following links (read-only click, editable Ctrl/Cmd+click; router, fragments, new tab) | S | C5 | L2 editor::links::ctrl_click_follows_plain_click_caret; L3 flows::links::item_link_opens_and_back | changed: no browser-native modifier clicks on the canvas |
| P-091 | A pasted same-instance item URL becomes a titled item link | S | C5 | L2 editor::links::pasted_item_url_becomes_titled_link; L3 flows::links::desktop_own_url_paste | same (new); on the desktop "same instance" is a `wikanvas://item/` link or its own listener's origin (§19.7.6) |
| P-092 | Link entry popup; empty removes; refused outside the allow-list with the message | S | C5 | L2 editor::links::popup_refuses_disallowed | same (new) |
| P-093 | To-do list: nested, clickable checkboxes, `[] ` rule, Mod+Shift+9 | M | C5 | L2 editor::blocks::task_checkbox_toggle_keeps_caret; L1 editor::input_rules::tests::task | same |
| P-094 | Tables: header row, Tab and Shift+Tab, delete when all selected, the table menu, slash "Table (3×3)" (weighed by sub-items: 16.5 days) | L | C5 | L2 editor::tables::* (the S0 G7 set) | same |
| P-094.1 | `TableMap` and the table's layout and grid painting | M | C5 | L1 editor::tables::map::tests | sub-item (a port of prosemirror-tables, R20) |
| P-094.2 | `CellSelection` (drag across cells, all cells) | M | C5 | L2 editor::tables::cell_selection | sub-item |
| P-094.3 | Row and column insert and delete | M | C5 | L2 editor::tables::rows_and_columns | sub-item |
| P-094.4 | Tab and Shift+Tab, a new row after the last cell | S | C5 | L2 editor::tables::tab_navigation | sub-item |
| P-094.5 | Header row toggle | S | C5 | L2 editor::tables::header_toggle | sub-item |
| P-094.6 | Backspace over all cells deletes the table; over some clears them | S | C5 | L2 editor::tables::delete_selected | sub-item |
| P-094.7 | The table menu | M | C5 | L2 editor::menus::table_menu | sub-item |
| P-094.8 | `fixTables` normalisation after remote edits | M | C5 | L1 editor::tables::fix::tests | sub-item |
| P-094.9 | Spans (colspan, rowspan) kept through edits | M | C5 | L2 editor::tables::spans_kept | sub-item |
| P-095 | Image node: width 25/50/75/100 or natural, alt, caption | M | C5 | L2 editor::images::width_alt_caption | same |
| P-096 | External image: never fetched, a card with "Open link" | S | C5 | L2 editor::images::external_never_fetched | same |
| P-097 | Image upload by paste, drop or slash; progress placeholder; failure with Retry | M | C5 (C1, C8) | L3 flows::docs::paste_image_uploads_with_progress; L1 client::transport::tests::upload_progress; L5 B16 | same (`Transport::upload` with progress; `XmlHttpRequest` on the web, §19.4.1) |
| P-098 | Callout node: 4 variants, icon, variant select, `:::x ` rule, slash items | M | C5 | L2 editor::blocks::callout_variant_picker_and_rule | same |
| P-099 | Mentions: `@` search (8), chip with live title, trashed, deleted and unavailable states | M | C5 | L2 editor::mentions::search_insert_live_title_states | same |
| P-100 | Board embed node: page-0 preview card, title, link, states; never starts the engine | M | C5 | L2 editor::blocks::board_embed_card_states; L3 flows::boards::board_reload_and_doc_embed; L3 flows::boards::seeded_board_preview_on_demand (`TestServer` with the seed, "Welcome to Wikanvas", its board card holds a decoded raster while `preview_rev` is null) | changed: the preview is a resvg raster of page 0 (D123) |
| P-101 | Slash menu: 18 items, filter, closes after 3 dead characters, keys | M | C5 | L1 editor::slash::tests::filter (5); L2 editor::menus::slash_keys | same |
| P-102 | "Embed board…" picker, "New board", and a board row dropped from the sidebar (D145) | S | C5 | L3 flows::boards::board_reload_and_doc_embed; L3 flows::boards::sidebar_board_drop_embeds | same |
| P-103 | Markdown input rules (headings, lists, tasks, quote, code, rule, marks, callouts) | M | C4 | L1 editor::input_rules::tests (every rule) | same (hand-written matchers) |
| P-104 | TipTap's default editing shortcuts | S | C4 | L1 editor::keymap::tests::tiptap_defaults_table | same (new) |
| P-105 | Bubble menu on a text selection (marks, link, "Turn into"), not in code or images | M | C5 | L2 editor::menus::bubble_on_selection_not_in_code | same |
| P-106 | Block handle: drag to reorder; Turn into, Duplicate, Copy link (headings), Delete | M | C5 | L2 editor::menus::block_handle_drag_keeps_block_id; L2 editor::menus::copy_link_heading; L2 ui::items::desktop_copy_item_link_without_listener | same on the web; changed on the desktop: a heading's Copy link only while browser access is on (§19.7.6) |
| P-107 | Placeholder "Type / for commands" | S | C4 | L2 editor::typing::placeholder_on_empty | same (new) |
| P-108 | Title ↔ body (Enter into the body, Backspace at an empty body's start to the title) | S | C4 (C3) | L2 ui::items::header_enter_moves_to_body; L2 editor::typing::backspace_empty_body_to_title | same |
| P-109 | Heading anchors, Contents from 2 headings (docked ≥ 1280 px, else a popover), scroll spy | M | C5 | L1 editor::contents::tests (3); L2 ui::docs::contents_docked_at_1280 | same |
| P-110 | Read-only document with the Trash or Archive banner and Restore / Unarchive | S | C3 | L2 ui::docs::trashed_doc_read_only_banner | same (new) |
| P-111 | Nested list under the body with "+ Add a page" | S | C3 | L2 ui::docs::children_list_and_add_page | same (new) |
| P-112 | Exports: Markdown (server), HTML | S | C3 | L3 flows::docs::export_markdown; L7 api::read_view::export_html_self_contained | changed: HTML from the server's renderer (D113) |
| P-113 | Typography and block styles | M | C4 | L4 snapshot::doc_every_block_light, snapshot::doc_every_block_dark | changed: embedded Noto Sans instead of the system UI font; synthetic italics (D120) |
| P-114 | Print only the document | S | C7 | L7 api::read_view::print_css_present; L5 B10 | changed: printing is the read view's (owner decision) (new) |
| P-115 | Browser spell check | L | C4 | none until the owner decides | owner: §19.15 q12; none in M2 by default, 0 days (the 42nd item without a test today) |

## J. Document autosave

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-116 | Open: GET with origin `server`, then the outbox, queued | M | C1 | L1 client::doc_sync::tests::load_then_outbox_queued; L1 client::doc_sync::tests::react_entry_deleted_after_post; L5 B8 (the React outbox adopted) | same, plus the React outbox adopted and its entry deleted after the POST that carried it (§19.4.4) |
| P-117 | Blank-document hold: opening never writes | S | C1 | L2 editor::corpus::open_close_writes_nothing; L1 client::doc_sync::tests::first_content_sends | changed: nothing is written before the first keystroke (§19.5.1) |
| P-118 | Send: debounce 1 s, at most 5 s apart; `pm_json` every 30 s | S | C1 | L1 client::doc_sync::tests::debounce_1s_max_5s | changed: no `pm_json` (D118); the same timings on the desktop (§19.4.4) |
| P-119 | Outbox written at most every 250 ms and before each POST | S | C1 | L1 client::doc_sync::tests::outbox_throttle_and_before_post | same |
| P-120 | 409 `missing_deps` resends from the server's state vector | S | C1 | L3 flows::docs::missing_deps_recovers | same |
| P-121 | Failures: offline back-off 1, 2, 5, 10, 30 s, and at once on `online`; 401; other with Retry | S | C1 | L1 client::doc_sync::tests::backoff_sequence_and_states; L1 client::doc_sync::tests::online_retries_at_once; L5 B9 | same |
| P-122 | Converge on focus and visibility (`?since=`) | S | C1 | L1 client::doc_sync::tests::focus_pulls_since; L3 flows::docs::cli_write_reaches_open_doc_on_focus | same (new) |
| P-123 | Flush on hide, blur, route change, Mod+S; `pagehide` keepalive under 60 KB | S | C1 (C8) | L1 client::doc_sync::tests::keepalive_under_60kb; L5 B6 | same |
| P-124 | Pending registry; one failing flush never stops the others | S | C1 | L1 client::pending::tests (the 4 cases) | same |

## K. Boards: host

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-125 | Board page top bar: title crumb, Text doc, Embed page…, Star, Nested pages, Focus, ⋯ with exports after a flush | M | C6 | L2 board::page::top_bar_and_exports | changed: exports through rfd dialogs on the desktop (D131) |
| P-126 | One engine for every board; a switch waits for the previous save; events of other boards ignored | M | C6 | L2 board::page::switch_waits_for_previous_save; L2 board::page::old_board_events_ignored | changed: direct calls on `BoardView` (§19.6) |
| P-127 | Loading state with the page-0 preview | S | C6 | L2 board::page::opening_after_300ms | changed: no engine download; only the open delay remains (new) |
| P-128 | Save loop: 1.5 s quiet, at most 10 s; `If-Match`; `mark_saved(revision)`; flushes | M | C1 | L1 client::board_sync::tests (the cases of saveLoop.test.ts); L3 flows::boards::draw_rectangle_saves | same |
| P-129 | Save failures: 412 conflict, 413 limit, offline back-off, 401, other | S | C1 | L1 client::board_sync::tests::failures_412_413_offline_401 | same |
| P-130 | Conflict banner: "Load latest" or "Keep mine as a copy" | M | C6 | L3 flows::boards::two_clients_conflict | same |
| P-131 | Unsaved snapshot stored before each PUT; restore prompt (24-hour time) | M | C1 | L1 client::board_sync::tests::unsaved_snapshot_cycle; L1 client::board_sync::tests::snapshot_on_hide_pagehide_401_and_every_2s; L2 board::page::restore_prompt; L5 B6, B8 | changed: the outbox store (file or IndexedDB); also on hide, `pagehide`, 401 and every 2 s for boards up to 2 MiB; React snapshots read from `keyval-store`/`keyval` and deleted after Restore or Discard (§19.4.5) |
| P-132 | Large board warning above 15 MB | S | C6 | L2 board::page::warning_over_15mb | same |
| P-133 | The engine follows the item's title and read-only state | S | C6 | L2 board::page::title_and_read_only_follow | same (new) |
| P-134 | The engine follows the resolved theme; the background stays board content | S | C6 | L2 board::page::chrome_follows_theme | same |
| P-135 | Read-only fallback with a reason (no engine, unsupported browser, failed start, panic, phones) | M | C6 | L2 board::page::load_failure_shows_preview; L5 B8, B12, B13, B14 | changed: no "engine not installed" or download branch; no WebGL2 or WebAssembly, or a failed download → the boot loader's panel with the reading link (D139); phones → the read view (D130) |
| P-136 | Nested pages popover with "+ Add a page" | S | C6 | L2 board::page::nested_popover | same |
| P-137 | Focus: the board and its cards full screen | S | C6 | L2 board::page::focus_fills_window | changed: the board panel fills the window (web Fullscreen API through web-sys, UNVERIFIED) |
| P-138 | Keys in the board: Mod+K and Mod+. reach the app, double Esc leaves, Ctrl+S saves | S | C6 | IB tests_widget::host_shortcuts_consumed_first; L2 board::page::double_esc_leaves_canvas | same |
| P-139 | Menus, popovers, dialogs and toasts paint above the board | S | C6 | L2 board::page::menus_paint_above_board (`layer_id_at`) | same |
| P-140 | Engine identity header; format version check | S | C6 | L1 board::tests::engine_version_header | changed: `X-Wikanvas-Engine`; the format check is a compile-time identity (D124) |

## L. Board document embeds (cards)

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-141 | Card content over the engine's box, placed by the camera, clipped; no style panel and no text settings while a card is edited (§18.14's D97) | M | C6 | IB tests_widget::card_painter_rects_and_z_order; IE tests_embed (the style-panel twins); L2 board::cards::card_rect_follows_camera_each_frame | changed: drawn in the engine's painter, in z-order; no occluders (§19.6.3) |
| P-142 | Which cards are drawn (zoom ≥ 0.5, size, at most 24, hysteresis) | S | C6 | L1 board::cards::tests::thresholds (the cases of layout.test.ts) | same |
| P-143 | Static card body: the first 200 blocks, cached with its ETag, "Empty page" | M | C6 | L2 board::cards::static_body_first_200_blocks | changed: the editor in read-only mode (D109) |
| P-144 | Card header: icon, live title, "Double-click to edit", badges, save badge | S | C6 | L2 board::cards::header_title_badges_save_state | same |
| P-145 | Card states: loading, static, editing, trashed, archived, missing, error | S | C6 | L2 board::cards::states | same |
| P-146 | Start editing (double-click, Enter, "Edit text"): caret at the point, the previous session's flush awaited | L | C6 | L2 board::cards::double_click_edits_caret_at_point; L3 flows::board_embeds::edit_in_place | same |
| P-147 | End editing (Esc in the title or the body, a primary or secondary press outside, layout or read-only changes, a save refused as `read_only` or 404, target gone, leaving); the middle and extra buttons never end it (§18.14's D93, D94, D96) | M | C6 | IB tests_widget::press_outside_card_ends_editing; IB tests_widget::presses_inside_editing_card_stay_in_card; IB tests_widget::middle_button_pans_keeps_editing; L2 board::cards::esc_and_outside_press_end; L2 board::cards::refused_save_ends_editing | changed: the in-process ending rules (§19.6.3) |
| P-148 | Card editor variant: no block handle, anchors or board items; Backspace to the card title; Ctrl+wheel; Shift and Alt with the wheel pan the board while a card is edited (§18.14's D94); Ctrl+Z only the document | M | C6 | L2 board::cards::variant_without_handle_and_board_items; L2 board::cards::shift_alt_wheel_pans_while_editing | same |
| P-149 | "Text doc": a child document, a card at the view centre, editing starts | M | C6 | L3 flows::board_embeds::text_doc | same |
| P-150 | "Embed page…" dialog with title search | M | C6 | L2 board::page::embed_page_picker | same |
| P-151 | Drop a sidebar document on the board, titled from its own `ItemDetail`, live documents only (§18.14's D98) | M | C6 | L3 flows::board_embeds::sidebar_drag; L2 board::cards::drop_titles_from_detail | changed: an in-app drag (D132) |
| P-152 | Unreferenced "Text doc" documents discarded when leaving the board; one on the engine's clipboard is kept until no clipboard holds it (§18.14's D95) | S | C6 | L3 flows::board_embeds::unreferenced_text_docs_discarded; L2 board::cards::clipboard_keeps_text_doc | same |
| P-153 | Title snapshots sent 250 ms after a title change, saved with the next board save; at most 200 characters (§18.14's D98) | S | C6 | L1 board::titles::tests (4, plus the 200-character cut) | same |
| P-154 | Cards are paper (light palette, white box with `#c4cad4` border) | S | C6 | L4 snapshot::board_cards_dark_theme | same |

## M. Search and quick switcher

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-155 | Search page: query, filters, sort, results with snippet, hint, Load more | M | C3 | L3 flows::search::search_and_switcher; L2 ui::search::filters_and_load_more | same |
| P-156 | Snippet marks, text only, U+E000/U+E001 handled, neighbours merged | S | C3 | L1 app::search::snippet::tests (3) | same |
| P-157 | Quick switcher (Mod+K): recent when empty, title search (120 ms), keys, mouse | M | C3 | L2 ui::switcher::* (the 5 cases) | same |

## N. Settings, tokens, data, about

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-158 | Settings shell: six sections, unknown is "Page not found" | S | C3 | L2 ui::settings::sections_and_unknown | same |
| P-159 | Profile: only changed fields sent; "Saved" | S | C3 | L2 ui::settings::profile_sends_changed_fields | same (new) |
| P-160 | Password change; forced mode | S | C3 | L3 flows::auth::force_password_change | changed: the desktop owner sets a first password here (§19.8.5) |
| P-161 | Sessions: list, sign out one, sign out all others | S | C3 | L2 ui::settings::sessions_sign_out_others | same (new) |
| P-162 | Appearance: Light, Dark, System, saved on the server | S | C3 | L2 ui::settings::theme_applies_and_saves | same (new) |
| P-163 | Workspace: name, logo upload and removal | S | C3 | L2 ui::settings::workspace_logo_upload_remove | same (new) |
| P-164 | API tokens list with scopes, dates, Revoke; the CLI note | M | C3 | L2 ui::settings::tokens_list_and_revoke | changed: the note names `wikanvas-cli` |
| P-165 | New token dialog; the secret shown once with Copy | M | C3 | L3 flows::settings::api_token_lifecycle | same |
| P-166 | Data: export the workspace, back up now, the backup note | S | C3 (C8) | L3 flows::settings::export_tar_and_backup (a file sink); L5 B7 | changed: a native save dialog on the desktop (D131) |
| P-167 | About: versions, licence list, engine version | S | C3 (C8) | L2 ui::settings::about_versions_and_licences; the image smoke (web-sys and wasm-bindgen listed) | changed: the licence list of the crates linked for Linux and for wasm32, Inkboard's lists, the fonts and the icons (§19.13); the engine version compiled in |

## O. Trash

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-168 | Trash page: collections and roots, Restore, Delete forever, Empty Trash, purge countdowns | M | C3 | L3 flows::trash::trash_and_restore; L2 ui::trash::purge_and_empty | same |

## P. Offline and caching

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-169 | Local persistence (outbox, unsaved boards, UI preferences), surviving full or disabled storage | M | C1 | L1 client::outbox::tests::survives_full_storage; L5 B8 | changed: files on the desktop, IndexedDB through web-sys, eframe storage for preferences |
| P-170 | Server-state cache: keys, optimistic updates, rollback, invalidation, retries | L | C1 | L1 client::cache::tests (optimistic, rollback, invalidation, retries) | same |
| P-171 | Conditional requests: tree and `docJson` ETags | S | C1 | L1 client::cache::tests::etag_304_keeps | same |

## Q. Keyboard shortcuts

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-172 | Global keys: Mod+K, Mod+., Mod+S, `?`, Esc; Mod+Shift chords do nothing | S | C2 (C8) | L2 ui::hotkeys::* (the 4 cases); L5 B15 | same; on the web the browser's own action for these chords is suppressed (D140) |
| P-173 | Shortcuts dialog (Mod shown as ⌘ or Ctrl) | S | C2 | L2 ui::dialogs::shortcuts_groups | same |
| P-174 | Menu keys (↓ opens, arrows wrap, Home/End, Enter/Space, Esc, Tab, accelerators) | S | C2 | L2 ui::menus::keyboard | same (new) |

## R. Dialogs and popovers

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-175 | Modal dialog: backdrop and Esc close, focus trapped and restored, stacking | S | C2 | L2 ui::dialogs::modal_focus_and_esc_stacking | same |
| P-176 | Confirmation; a new question cancels the open one | S | C2 | L2 ui::dialogs::confirm_new_question_cancels_old | same |
| P-177 | Menu placement: below, else above, else the roomier side with a capped height | S | C2 | L2 ui::menus::placement_flips_and_caps_height; L3 flows::links::low_row_menu_opens_upward | same (own placement code if egui's differs, UNVERIFIED) |

## S. Notifications and errors

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-178 | Toasts: kinds, one action, 5 s / 8 s, at most 4, × | S | C2 | L2 ui::toasts::limits_and_action | same |
| P-179 | Error model and wording | S | C2 | L1 client::transport::tests::error_envelope; L1 app::messages::tests | same |
| P-180 | Inline form errors from `details.field` | S | C2 | L2 ui::dialogs::form_field_error_inline | same |
| P-181 | Empty and failure states of pages | S | C2 | L2 ui::shell::not_found_and_empty_texts | same |

## T. Theming and layout tokens

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-182 | Design tokens (15 colours light and dark, focus ring, fonts, type scale, spacing, radii, motion) | M | C2 | L1 app::theme::tests::tokens_match_9_6_1; L4 snapshot::sidebar_light, snapshot::sidebar_dark | changed: egui `Visuals` and named colours (§19.10) |
| P-183 | Theme light, dark, system; the server's choice wins | S | C2 | L2 ui::theme::system_follows_os_server_wins | changed: no pre-paint script; eframe's system theme |
| P-184 | Reduced motion | S | C2 | L2 ui::theme::reduced_motion_zero_animation | changed: web from `matchMedia`, desktop a setting (new) |
| P-185 | 12 collection colours and 48 icons | M | C2 | L1 app::icons::tests::collection_icon_set_complete | changed: resvg-rendered SVGs (D133) |
| P-186 | 35 UI icons | S | C2 | L1 app::icons::tests::all_ui_icons_embedded | changed: resvg-rendered SVGs (new) |
| P-187 | Breakpoints (1024 drawer, 1280 Contents, phones) | S | C2 | L2 ui::shell::breakpoints_1024_1280; L5 B12 | changed: phones open the read view (D130) |

## U. Internationalisation

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-188 | English only; dates in `en`; Inkboard follows the browser language | S | C2 | L1 app::i18n::tests::inkboard_follows_app_language | changed: one locale source (D121) (new) |

## V. Accessibility affordances

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-189 | Landmarks and focus targets (main, sidebar, navs) | S | C2 | L2 ui::a11y::landmarks_and_esc_focus_main | changed: AccessKit roles (desktop); on the web for tests only |
| P-190 | Tree semantics (level, position, expanded, selected, current, one tab stop) | M | C2 | L2 ui::a11y::tree_item_level_expanded_selected | changed: AccessKit tree roles |
| P-191 | Live and status regions | S | C2 | L2 ui::a11y::status_and_alert_nodes | changed: AccessKit live settings (new) |
| P-192 | Names on every control | S | C2 | L2 ui::a11y::label_audit_every_screen | same (new: the label audit) |
| P-193 | Boards for screen readers: "Text on this board"; the editing card as a region | S | C6 | L2 board::page::text_on_board_hidden_list | changed: AccessKit nodes (desktop) |
| P-194 | Visible focus, everything reachable by keyboard | S | C2 | L4 snapshot::focus_ring | same (new) |

## W. Features the browser gave the SPA

| ID | Behaviour | Size | WS | New test(s) | Port |
|---|---|---|---|---|---|
| P-195 | Find in page | M | C5 | L2 editor::find::mod_f_highlights_hits; L5 B10, B15 | changed: in-editor find with the browser's Mod+F suppressed (D135, D140); the browser's find on the read view (new) |
| P-196 | Password managers and autofill on sign-in, setup, the password change (current and new) and the admin token's password prompt | M | C7 | L7 api::pages::forms_autocomplete_attributes | changed: sign-in and setup are real HTML forms (D114); the password change and the admin token's password prompt (`web/src/pages/settings/Account.tsx:85-93`, `ApiTokens.tsx:155`) become canvas fields, so the web loses autofill there (a server form `/account/password` is a later option) (new) |
| P-197 | Page zoom and text scaling | S | C2 | L2 ui::zoom::ctrl_plus_zooms_board_when_focused_else_ui | changed: D122 (new) |
| P-198 | Select and copy any text | S | C2 | L2 ui::shell::labels_selectable | changed: selectable labels, plain-text copy (new) |
| P-199 | Native link context menu ("Open in new tab", "Copy link address") | M | C5 | L2 editor::links::context_menu_new_tab_and_copy_address | changed: Wikanvas's own context menu |
| P-200 | Rich clipboard between applications | M | C4 | L2 editor::clipboard::html_round_trip; L5 B11 | same (D112) (new) |
| P-201 | Back and forward (mouse buttons, Alt+←/→), reload restores the page | S | C8 | L5 B5; L2 ui::routing::desktop_history_back_forward | changed: web History; desktop in-app history (new) |
| P-202 | Page translation and reader modes | S | C7 | L5 B10 | changed: the read view (new) |

## Old tests → new tests

Every old test maps to a new test or a written reason. The ledger's parity gate also covers these rows.

**Smoke steps** (`e2e/tests/smoke.spec.ts`, 1,045 lines at `9c6e60a`):

| Step | New test(s) |
|---|---|
| 1. Setup, Home shows "Getting started" | L7 api::pages::setup_form_creates_owner_and_redirects; L3 flows::setup::desktop_first_run (on `TestServer::start_unprovisioned` with the seed); L5 B1 |
| 2. Sign out, sign in with "Keep me signed in" | L3 flows::auth::sign_out_clears_cache; L5 B2 |
| 3. Eng / A / B / C, "# Plan" and "quokka", Saved, reload | L3 flows::items::nested_docs_and_reload (`restart()` is the reload); L5 B3 |
| 4. Delete the body, empty after reload | L3 flows::docs::delete_body_then_retype |
| 5. Search, `-word`, Mod+K | L3 flows::search::search_and_switcher |
| 6. Star C; no Publish, no Drafts | L3 flows::items::star_and_no_drafts |
| 7. Move B with its subtree | L3 flows::items::move_dialog |
| 8. Trash B, "+1 nested", Restore | L3 flows::trash::trash_and_restore |
| 9. Board under A; menus above the canvas; the fallback when WebGL 2 is missing (`e2e/tests/smoke.spec.ts:537-548`) | L3 flows::boards::new_board_menus_above_canvas (`layer_id_at`); L5 B4; the fallback → L5 B13 (the boot loader's panel, D139) |
| 10. Draw a rectangle, Saved | L3 flows::boards::draw_rectangle_saves |
| 11. Reload keeps it; doc card loads no engine | L3 flows::boards::board_reload_and_doc_embed; "no `/inkboard/` request" is gone (no separate engine download) |
| 12. Two pages, conflict banner | L3 flows::boards::two_clients_conflict (two apps, one server) |
| 13. Export is a tar; CSRF without metadata | L5 B7; L3 flows::settings::export_tar_and_backup |
| 14. API token shown once; Bearer; revoke | L3 flows::settings::api_token_lifecycle |
| 15. Export Markdown | L3 flows::docs::export_markdown; L5 B7 |
| 16. Recent; item links; Back; Welcome board preview; low row menu upward | L3 flows::lists::recent_after_view, flows::links::item_link_opens_and_back, flows::links::low_row_menu_opens_upward, flows::boards::seeded_board_preview_on_demand (the seeded Welcome board's card loads a preview rendered on demand while `preview_rev` is null, `e2e/tests/smoke.spec.ts:770-782`); L5 B5 |
| 17. Board document embeds end to end, with the M1.1 review's additions (Shift and Alt with the wheel and the middle button pan while C is edited, Esc in C's body) | L3 flows::board_embeds::* at 1600 × 1000, flows::board_embeds::pan_while_editing, ::esc_in_body; "the overlay never scrolls" is gone (no DOM overlay), replaced by "the card's rect stays on its box" from AccessKit bounds |
| CSP check | L5 B1 (app), B10 (read view) |

**Frame tests** (`e2e/tests/frame.spec.ts`, 26 at `9c6e60a`, 12 of them `embeds:`): serving and loading → B1 and the
image smoke; snapshot round trip → IB tests_widget::snapshot_round_trip; draw, onChange, markSaved, Ctrl+S, host keys →
L2 board::page::dirty_saved_ctrl_s and IB tests_widget::host_shortcuts_consumed_first; read-only blocks the drag → L2
board::page::read_only; `location.hash` → gone (no frame page); setBoard resets undo → L2
board::page::switch_resets_undo; an unreadable board keeps the previous one → L2
board::page::malformed_board_keeps_previous; one panic per trap → B8; `connect()` retried and the format check → gone
(no separate engine; the format is a compile-time identity, D124); setBoard while editing a label → L2
board::page::switch_while_editing_label; F5 does not present → IB tests_widget::present_not_bound; setActive(false)
stops repaints → L2 board::page::no_repaint_off_board and B4; hidden page → B6 (the 60 s background case stays a manual
check); the 12 `embeds:` tests → the IE tests_embed:: twins plus L2 board::cards::* ("while the host edits, Shift and
Alt with the wheel pan and the middle button pans, keeping the focus" → IB
tests_widget::middle_button_pans_keeps_editing and L2 board::cards::shift_alt_wheel_pans_while_editing), and "frame
pixels when the canvas is not at the frame origin" is gone (no frame-to-host conversion).

**Vitest** (79 files; 350 `it`/`test` call sites at `9c6e60a` by `grep -cE '^\s*(it|test)(\.each\([^)]*\))?\('`, 343 at
`d5d3c94`; 396 tests ran at the M1.1 gate, parametrised ones expanded, `docs/handoff/E.md:90`). One row per file; F0
expands each into one row per call site, each naming a new test `path::name` or `gone: <reason>` (PLAN M2 F0 task 6):

| File (`web/src/…`) | Call sites | New test(s) |
|---|---|---|
| `api/client.test.ts` | 7 | L1 client::transport::tests (error envelope, version check, cookie only, octet-stream) |
| `api/pages.test.ts` | 2 | L1 client::api::tests::page_cursor |
| `app/appShell.test.tsx` | 6 | L2 ui::shell::* |
| `app/AuthGate.test.tsx` | 6 | L2 ui::auth::gate_states, ui::auth::lost_session_saves_outbox_then_navigates |
| `app/hotkeys.test.ts` | 4 | L2 ui::hotkeys::* |
| `app/router.test.tsx` | 5 | L1 app::route::tests::*; L2 ui::routing::* |
| `app/theme-init.test.ts` | 4 | gone: no pre-paint script (P-183); L2 ui::theme::system_follows_os_server_wins keeps the outcome |
| `app/updateBanner.test.tsx` | 3 | L2 ui::shell::update_banner_once_on_build_mismatch |
| `board/boardPage.test.tsx` | 9 | L2 board::page::* |
| `board/conflict.test.tsx` | 2 | L2 board::page::conflict_banner_choices; L3 flows::boards::two_clients_conflict |
| `board/embed/css.test.ts` | 4 | gone: no DOM overlay stylesheet |
| `board/embed/drop.test.tsx` | 4 | L2 board::cards::drop_titles_from_detail; L3 flows::board_embeds::sidebar_drag |
| `board/embed/editing.test.tsx` | 19 | L2 board::cards::* (start, end, Esc, refused save, focus back); IB tests_widget::press_outside_card_ends_editing, ::presses_inside_editing_card_stay_in_card |
| `board/embed/embedLayer.test.tsx` | 5 | L2 board::page::old_board_events_ignored; the rest gone: no overlay layer, `inert` or `fullscreenchange` move |
| `board/embed/layout.test.ts` | 14 | L1 board::cards::tests::thresholds (`chooseCards`, hysteresis, the cap of 24); world → frame → viewport and `clipPath` gone |
| `board/embed/picker.test.tsx` | 2 | L2 board::page::embed_page_picker |
| `board/embed/staticCard.test.tsx` | 8 | L2 board::cards::static_body_first_200_blocks, ::states, ::header_title_badges_save_state |
| `board/embed/textDoc.test.tsx` | 10 | L3 flows::board_embeds::text_doc, ::unreferenced_text_docs_discarded; L2 board::cards::clipboard_keeps_text_doc |
| `board/embed/titles.test.ts` | 4 | L1 board::titles::tests |
| `board/fallback.test.tsx` | 5 | L2 board::page::load_failure_shows_preview; L5 B8, B12, B13 |
| `board/frameLifecycle.test.tsx` | 4 | gone: no iframe |
| `board/saveLoop.test.ts` | 10 | L1 client::board_sync::tests |
| `board/staleEvents.test.ts` | 2 | L2 board::page::old_board_events_ignored |
| `board/unsaved.test.ts` | 5 | L1 client::board_sync::tests::unsaved_snapshot_cycle; L1 client::outbox::tests |
| `board/warmup.test.ts` | 2 | gone: no engine download |
| `board/warmupWiring.test.tsx` | 1 | gone: no engine download |
| `components/item/contract.test.tsx` | 4 | L2 ui::items::header_* |
| `components/item/createdState.test.tsx` | 1 | L2 ui::items::create_opens_created_here |
| `components/item/fullWidth.test.tsx` | 3 | L2 ui::items::full_width_optimistic_rollback |
| `components/item/titleAutosave.test.tsx` | 4 | L2 ui::items::title_autosave_debounce_and_retry |
| `components/overlays/collectionDialog.test.tsx` | 3 | L2 ui::collections::dialog_create_edit_preview |
| `components/overlays/moveDialog.test.tsx` | 3 | L2 ui::items::move_dialog_filters_and_excludes_subtree |
| `components/quickSwitcher/quickSwitcher.test.tsx` | 5 | L2 ui::switcher::* |
| `components/search/snippet.test.tsx` | 3 | L1 app::search::snippet::tests |
| `components/sidebar/autoExpand.test.ts` | 4 | L1 app::tree::auto_expand::tests |
| `components/sidebar/collectionDrop.test.tsx` | 2 | L2 ui::sidebar::drop_on_collection_moves_subtree |
| `components/sidebar/collectionReorder.test.tsx` | 3 | L2 ui::sidebar::collection_reorder |
| `components/sidebar/currentItem.test.tsx` | 2 | L2 ui::sidebar::current_item_expands_ancestors_once |
| `components/sidebar/dragExternal.test.tsx` | 1 | gone: no cross-frame `DataTransfer`; L2 ui::sidebar::doc_row_carries_item_payload |
| `components/sidebar/expandedPersistence.test.ts` | 4 | L1 app::tree::expanded::tests |
| `components/sidebar/noDraftUi.test.tsx` | 3 | L2 ui::sidebar::nav_has_no_drafts |
| `components/sidebar/optimisticMove.test.tsx` | 3 | L2 ui::sidebar::optimistic_move_rolls_back_on_409 |
| `components/sidebar/starReorder.test.tsx` | 3 | L2 ui::sidebar::star_reorder_sends_neighbours, ::star_reorder_rolls_back |
| `components/sidebar/virtualRows.test.tsx` | 1 | L2 ui::sidebar::ten_thousand_rows_lays_out_visible_only |
| `components/tree/buildTree.test.ts` | 7 | L1 app::tree::build::tests |
| `components/tree/hitZones.test.ts` | 10 | L1 app::tree::hit_zones::tests |
| `components/tree/invalidTargets.test.ts` | 4 | L1 app::tree::drop::tests::invalid_targets |
| `components/tree/keyboard.test.tsx` | 11 | L2 ui::sidebar::keyboard_navigation |
| `components/tree/nonManualDrop.test.ts` | 4 | L1 app::tree::drop::tests::non_manual |
| `components/tree/positions.test.ts` | 10 | L1 app::tree::positions::tests |
| `components/tree/visibleRows.test.ts` | 6 | L1 app::tree::visible_rows::tests |
| `components/ui/menuPlacement.test.ts` | 4 | L2 ui::menus::placement_flips_and_caps_height |
| `editor/baseStyles.test.ts` | 1 | L4 snapshot::doc_every_block_light |
| `editor/boardEmbed.test.tsx` | 2 | L2 editor::blocks::board_embed_card_states |
| `editor/contents.test.ts` | 3 | L1 editor::contents::tests |
| `editor/docSession.test.ts` | 11 | L1 client::doc_sync::tests |
| `editor/embedVariant.test.tsx` | 4 | L2 board::cards::variant_without_handle_and_board_items |
| `editor/exportMarkdown.test.ts` | 2 | L3 flows::docs::export_markdown |
| `editor/externalImage.test.tsx` | 3 | L2 editor::images::external_never_fetched |
| `editor/fixtures/ydoc.test.ts` | 2 | L2 editor::corpus::reads_every_fixture (the corpus stays, frozen at X) |
| `editor/inputRules.test.ts` | 4 | L1 editor::input_rules::tests |
| `editor/linkOpen.test.ts` | 4 | L2 editor::links::ctrl_click_follows_plain_click_caret |
| `editor/link.test.ts` | 3 | L1 editor::links::tests::allow_list |
| `editor/mention.test.tsx` | 3 | L2 editor::mentions::search_insert_live_title_states |
| `editor/schema.test.ts` | 3 | L1 editor::model::tests::names_match_schema_v1 |
| `editor/serverWritten.test.ts` | 2 | L2 editor::corpus::reads_every_fixture (the server-written directory) |
| `editor/slash.test.tsx` | 5 | L1 editor::slash::tests::filter; L2 editor::menus::slash_keys |
| `editor/uniqueId.test.ts` | 1 | L1 editor::commands::tests::new_blocks_get_ids |
| `editor/upload.test.ts` | 2 | L1 client::transport::tests::upload_progress; L5 B16 |
| `lib/time.test.ts` | 2 | L1 app::time::tests |
| `lib/urls.test.ts` | 8 | L1 app::route::tests::next_param_rules; L1 core::convert::tests::item_ref (exist) |
| `pages/docPage.test.tsx` | 4 | L2 ui::docs::* |
| `pages/settings/about.test.tsx` | 4 | L2 ui::settings::about_versions_and_licences |
| `pages/settings/apiTokens.test.tsx` | 5 | L2 ui::settings::tokens_list_and_revoke; L3 flows::settings::api_token_lifecycle |
| `pages/setup.test.tsx` | 2 | L7 api::pages::setup_form_creates_owner_and_redirects; L3 flows::setup::desktop_first_run |
| `pages/trash.test.tsx` | 4 | L2 ui::trash::purge_and_empty |
| `state/pending.test.ts` | 4 | L1 client::pending::tests |
| `state/saveStatus.test.ts` | 4 | L1 client::pending::tests::save_status_states |
| `styles/layers.test.ts` | 3 | L3 flows::boards::new_board_menus_above_canvas; L3 flows::links::low_row_menu_opens_upward |
## New behaviour (M2)

Behaviours the React client never had, so no item above describes them. Each needs its named test; `scripts/parity.sh`
fails on a `missing` row here too. N-04 to N-07 are not used: they were fallbacks for the old names, and the rename
(D100) kept none.

| ID | Behaviour | WS | Test |
|---|---|---|---|
| N-01 | `/login/once`: a 32-byte ticket, single use, 60 s lifetime, then a browser session and a redirect | C7 | L7 api::pages::login_once_single_use_and_expiry |
| N-02 | The desktop agent token: revoked at a clean exit and at the next start | C7 | L7 api::desktop::agent_token_revoked_at_exit_and_next_start |
| N-03 | The CLI refuses a discovery file not owned by the user, not 0600, or with a dead pid | C7 | L1 cli::config::tests::discovery_file_refusals |
| N-08 | A second desktop instance on the same data dir is refused | F0, C8 | L3 flows::desktop::second_instance_refused |
| N-09 | No schema change: `PRAGMA user_version` stays 1 after an M2 run (D143) | F0 | L1 store::schema::tests::user_version_stays_1 |
| N-10 | The desktop session is minted again after a 401 (D105) | C1 | L3 flows::client::desktop_session_survives_password_change |
| N-11 | Saves fire while the desktop window is minimised (D106) | C1 | L3 flows::client::minimised_window_still_saves |
| N-12 | The boot loader: feature checks, the phone check, one retry, the error panel (D139) | C8 | L5 B12, B13, B14 |
| N-13 | The browser's own action for an app shortcut is suppressed (D140) | C8 (C2's table) | L5 B15 |
| N-14 | Assets under `/app/{build}/`; another build id is 404 (D141) | C7 | L7 api::serve::app_assets_by_build; the image smoke |
| N-15 | The desktop's log file, crash note and restore dialog | C8 | L3 flows::desktop::panic_writes_crash_note_and_restore_dialog |
| N-16 | The desktop listener keeps its port across restarts | C8 | L3 flows::desktop::listener_port_kept_across_restart |
| N-17 | "Copy item link" on the desktop while browser access is off | C3 | L2 ui::items::desktop_copy_item_link_without_listener |
| N-18 | `serve`: the LAN refusal, the data-dir lock, the port in the cookie name, the setup link | C7 | L7 api::serve::* |
| N-19 | Unique heading anchors in the read view when a `blockId` repeats | C7 | L7 api::read_view::duplicate_block_ids_unique_anchors |
| N-20 | The later of two blocks sharing a `blockId` gets a new one when edited | C4 | L1 editor::commands::tests::duplicate_block_id_renewed_on_edit |
| N-21 | The licence list covers the wasm32 crates, Inkboard's lists, the fonts and the icons | C8 | the image smoke; L2 ui::settings::about_versions_and_licences |

## Manual checks

ARCHITECTURE §16 items 26 and 31 list checks that M1 left manual (`docs/ARCHITECTURE.md:3311-3319`, `:3358-3376` at
`484905c`). In one egui UI most of them become kittest scenarios; the rest are recorded by hand. C9 owns this table; I
records each manual result (browser, version, date, pass or fail) in `docs/handoff/M2-merge.md` before X, and X waits
for them.

| Check (from §16) | M2 |
|---|---|
| Fullscreen (Focus) | L2 board::page::focus_fills_window; the web Fullscreen API by hand in Chromium, Firefox and Safari |
| Double Esc out of the canvas | L2 board::page::double_esc_leaves_canvas |
| A background tab (headless Chromium reports it visible) | B6 (emulated); a 60 s real background tab by hand in Chromium and Firefox |
| The editor's bubble menu, block handle and popover placement | L2 editor::menus::bubble_on_selection_not_in_code, ::block_handle_drag_keeps_block_id; L2 ui::menus::placement_flips_and_caps_height |
| A real drag and drop in the sidebar | L2 ui::sidebar::* (kittest pointer drags); by hand in Firefox and Safari |
| The About page | L2 ui::settings::about_versions_and_licences |
| The real drag and the `focus` end in Firefox and Safari | by hand (the in-app drag of D132; the focus end of §19.6.3) |
| A drop from another window on the editing card | by hand on the desktop and in Chromium (OS file drops reach eframe as dropped files) |
| The cards on their boxes during a pan, a zoom or a drag | L2 board::cards::card_rect_follows_camera_each_frame (the painter draws them in the same frame) |
| The clip around the engine's menus; a closing popup's one-frame lag | L2 board::page::menus_paint_above_board; the lag is gone (no overlay) |
| Crisp card text after zooming in | S0 G15; L4 snapshot::card_text_zoom_3 |
| A pinch (Ctrl+wheel) over the editing card not zooming the page | by hand with a trackpad in Chromium, Firefox and Safari |
| The slash, mention, bubble and table menus of a card, also in Focus | L2 board::cards::menus_in_card, ::menus_in_card_focus |
| The picker's focus order | L2 board::page::embed_page_picker (focus returns to the canvas) |
| IME composition in a card | L2 board::cards::ime_in_card (synthetic `Event::Ime`); by hand with ibus or fcitx5 |
| A rotated card | L2 board::cards::rotated_card_shows_engine_box (owner question 10) |
| A background tab with a card editing | by hand in Chromium |
| Firefox and Safari: the middle-button pan's focus | by hand |
| A trackpad's horizontal scroll while a card is edited | by hand on the desktop and in Chromium |
| Screen readers on the desktop (NVDA, Orca, VoiceOver) | by hand, recorded, not gated (§19.5.8) |

**Supported browsers** for the editing app: current Chromium-based browsers (Chrome, Edge, Chromium), gated by L5 in the
pinned Playwright image's Chromium; current Firefox and Safari 17 or newer, checked by hand with the rows above before
X; phones open the read view by default (D130, B12), and the editor there is best effort. Every browser gets the read
view, which needs no script.

## API calls

The client calls 59 method and path pairs under `/api/v1` (56 rows of the inventory). `wikanvas-client::api` has a typed
method for all 70 operations of `docs/api/openapi.json`; each pair the client uses is exercised by at least one L3 flow:

| Calls | Exercised by |
|---|---|
| `GET /config`, `GET`/`POST /setup`, `POST /auth/login`, `POST /auth/logout`, `GET`/`PATCH /auth/me`, `POST /auth/password`, `GET /auth/sessions`, `DELETE /auth/sessions/{id}`, `DELETE /auth/sessions?others=true` | flows::auth::*, flows::setup::*, flows::settings::* |
| `GET`/`POST /tokens`, `DELETE /tokens/{id}` | flows::settings::api_token_lifecycle |
| `PATCH /workspace`, `POST /attachments`, `GET /attachments/{id}` | flows::settings::workspace_logo, flows::docs::paste_image_uploads_with_progress |
| `GET /sidebar`, `GET /collections/{id}/tree`, `GET`/`POST`/`PATCH`/`DELETE /collections/{id}`, `POST /collections/{id}/move`, `POST /collections/{id}/restore` | flows::items::*, flows::trash::* |
| `POST /items`, `GET`/`PATCH`/`DELETE /items/{id}` (also `?discard_empty=true`), `GET …/children`, `POST …/move`, `…/duplicate`, `…/archive`, `…/unarchive`, `…/restore`, `PUT`/`DELETE …/star`, `POST …/view` | flows::items::*, flows::lists::*, flows::trash::* |
| `GET /recent`, `/updated`, `/starred`, `/archive`, `/trash`, `DELETE /trash/items/{id}`, `DELETE /trash/collections/{id}`, `DELETE /trash` | flows::lists::*, flows::trash::* |
| `GET /search`, `GET /search/titles` | flows::search::* |
| `GET …/doc` (and `?since=`), `POST …/doc/updates`, `GET …/doc/markdown?download=true`, `GET …/doc/prosemirror` | flows::docs::*, flows::board_embeds::* |
| `GET`/`PUT …/board`, `GET …/board.inkboard`, `GET …/board/pages/{n}` | flows::boards::* |
| `GET /export`, `POST /admin/backup` | flows::settings::export_tar_and_backup, B7 |
| New: `GET …/doc/html` | L7 api::read_view::*, flows::docs::copy_as_html |
