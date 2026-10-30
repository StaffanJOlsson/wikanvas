# PlugWiki: brief for an architectural feasibility review

Written 2026-10-30 for a reviewing agent. Status: idea, not a decision. Nothing here changes the binding design in
`docs/ARCHITECTURE.md` until the owner accepts it.

---

## 0. Your task

Judge whether the idea below is **architecturally feasible**. Do **not** estimate time or effort.

What to produce:
1. A **verdict** for each design question in §5: is it feasible, which option you recommend, and why. Name what would
   make it infeasible.
2. The **show-stoppers**, if any: things that cannot work under the constraints in §2 and §8, or only by giving up a
   goal. Say which goal.
3. A **recommended architecture**, at the level of crates, interfaces and data flow. Say where it reuses today's code
   and where it replaces it.
4. The **spikes** that would settle the open risks: small prototypes, each with a pass/fail criterion.
5. The **questions only the owner can answer**, kept apart from your own conclusions.

Ground rules:
- Read the code and `docs/ARCHITECTURE.md` (especially §1, §4–§7, §19) before trusting this brief. Where the brief and
  the repository disagree, the repository wins: say so in your report.
- §7 and §9 come from a web search on 2026-10-30 and are **unverified** in places. Check what your conclusions depend
  on.
- This is a review. Do not change code, do not build, and do not open pull requests. If you must run something, follow
  `CLAUDE.md` (§8 below).

---

## 1. The idea

**PlugWiki** is the planned new name and direction for Wikanvas: a self-hosted, open-source, multi-user wiki where
**nearly everything is a plugin**. The core holds only what every content type shares. Each content type and
integration is a plugin:
- the sidebar tree of collections and items;
- accounts and access;
- storage, history, search and links;
- sync;
- the API and CLI.

Planned plugins:
- **Documents**: today's rich-text editor, stored as Yjs.
- **Boards**: today's Inkboard whiteboards.
- **Sheets**: new, an Excel-like table sheet.
- **GitHub integration**: new, scope still open (see §6.2).
- Later, plugins by others: an owner question (§10).

Why the owner wants it:
- **Distinct.** It's an open-source, self-hosted, multi-user answer to Obsidian's plugin model. Obsidian is closed
  source and single-user.
- **Easier for one maintainer.** Each feature lives behind one interface, instead of the doc/board special cases
  running through every layer.
- **Rust "for pretty much everything"**, in line with the M2 decision (§3.3).

---

## 2. Goals and constraints from the owner

| # | Goal or constraint | Source |
|---|---|---|
| G1 | Everything, or nearly everything, is a plugin; the core is small | owner, 2026-10-30 |
| G2 | Rust for pretty much everything: server, client, plugins | owner, 2026-10-30; M2 decisions 2026-10-29 (§3.3) |
| G3 | Self-hosted, multi-user, access per collection (already built, §3.2) | ARCHITECTURE §1 |
| G4 | Open source, MIT. Dependencies only MIT, Apache-2.0, BSD, ISC, Zlib, Unicode or CC0 (build-only tools may be MPL-2.0). No code from AGPL or BSL projects | CLAUDE.md, PLAN R6 |
| G5 | The live wiki must never lose data. Stored formats change only with a migration path; documents stay Yjs, boards stay the Inkboard envelope | ARCHITECTURE §6, D143 |
| G6 | One maintainer: the design must lower the support burden, not raise it | owner |
| G7 | Small runtime footprint: the app container runs at 2 CPUs and 1 GiB, no swap | `compose.yaml` |
| G8 | A public, versioned API (`/api/v1`, additive only) and a CLI that agents use | ARCHITECTURE §7 |

---

## 3. What exists today (Wikanvas)

### 3.1 Stack and size (main, 2026-10-30)

- **Server:** Rust 2024 (toolchain 1.99.1). axum 0.8.9, SQLite through rusqlite 0.40.2, yrs 0.28 (the Rust port of
  Yjs), utoipa for OpenAPI 3.1. One binary, `wikanvas-server`.
- **Client:** React 19 + TypeScript + TipTap 3 (about 27,000 lines in `web/src`).
- **Boards:** Inkboard (`vendor/inkboard`, an egui app, MIT OR Apache-2.0), compiled to WebAssembly and patched by
  `patches/inkboard/0001–0006`. It runs in an iframe and talks to the page over a JS bridge.
- **Rust crates** (lines in `src/`):

  | Crate | Lines | Contents |
  |---|---|---|
  | `wikanvas-api` | 3,065 | DTOs shared by server, CLI and (via ts-rs) the web client |
  | `wikanvas-core` | 12,032 | Pure domain: ids, tree rules, Y walkers, Y ↔ ProseMirror ↔ Markdown conversion, board codec |
  | `wikanvas-store` | 9,037 | rusqlite repositories, blobs, revisions, backup |
  | `wikanvas-server` | 12,310 | Routes, auth, CSRF, jobs, OpenAPI |
  | `wikanvas-cli` | 6,907 | The `wikanvas` CLI, over HTTP only |
  | `wikanvas-assets` | 61 | rust-embed of the web dists |

- **Hosting:** Docker behind Caddy (HTTPS). CI on GitHub Actions. Changes reach `main` only through pull requests with
  green CI.

### 3.2 Data model and how content types are wired in

From `crates/wikanvas-store/migrations/0001-init/up.sql` and 0002–0003.

**Shared tables:**
- `items`: id, `collection_id`, `parent_id`, `kind`, title, `sort_key`, publish, archive and trash state.
- `collections`, plus access (`default_access`, `collection_members`).
- `revisions`, `item_links` (mention, link, embed), `item_text` (full-text search).
- `attachments` and `blobs`, `stars`, `item_views`.
- Users, sessions, tokens, invites.

**The two content types are hard-wired:**
- `item_kinds` holds exactly `doc` and `board`. `content_formats` holds exactly `yjs` and `inkboard`.
  `ItemKind { Doc, Board }` is a closed enum in `wikanvas-api` (`crates/wikanvas-api/src/common.rs:98`).
- Each kind has its own tables:
  - documents: `doc_state`, `doc_updates` (an append-only Yjs update log) and `doc_clients`;
  - boards: `board_state`, `board_assets` and `board_previews`.
- Each kind has its own route modules (`routes/docs.rs`, `routes/boards.rs`) and CLI subcommands (`doc`, `board`),
  and needs its own code for:
  - text extraction for search and links (`ytext` for documents; `board::inspect` for boards);
  - previews (board SVG);
  - exports (Markdown and `.inkboard`).
- Cross-kind features are handled pairwise. Example: a document embedded as a card on a board (§18, M1.1, D58–D99)
  needed a JS bridge, an Inkboard patch (0005) and changes to both kinds' storage, text and access code.
  "Redact cards" leaves a board's card labels out where the viewer may not read the document.

**Sync today:**
- Documents use a pull protocol: debounced POSTs of merged Yjs updates and a GET of state (§9.4). There is no live
  push and no presence yet.
- Boards save whole snapshots, with conflict detection.

**Access:** checked in one place, `authz::require`. A viewer who may not read an item gets 404, and the item is left
out of every list, search and title lookup. A leak suite tests this. **Any plugin design must keep this guarantee.**

**Outbound network:** D29 says the server makes no outbound calls and links no TLS stack; HTTPS terminates in Caddy.
PLAN R6 also forbids `webpki-roots` (CDLA-Permissive-2.0).

### 3.3 M2: the all-Rust egui client (proposal, ARCHITECTURE §19)

M2 is the owner-approved direction of 2026-10-29. It is written as a proposal and not yet built. PlugWiki has to fit
with it, replace it, or be layered after it. Key points:
- One egui 0.37 app for desktop and browser (wasm32). Shell, document editor and board share one `egui::Context`: no
  iframe, no JS bridge, no DOM overlay (D101).
- A new rich-text editor in Rust (about 23,000 lines estimated) that edits the yrs `XmlFragment` directly, in the
  exact structure y-tiptap writes (D107), so stored documents don't change.
- Inkboard is hosted as a widget (`BoardView`) through a patched library target (D103).
- Proposed crates:
  - `wikanvas-client`: data layer, no egui, native and wasm;
  - `wikanvas-editor`: reaches the app only through an `EditorHost` trait;
  - `wikanvas-board`, `wikanvas-app`, `wikanvas-web` (the wasm entry);
  - one binary `wikanvas` (desktop mode with the server in-process, plus `serve`);
  - the CLI becomes `wikanvas-cli`.
- One API boundary: every client speaks `/api/v1` through a `Transport` (D104).
- Accepted costs:
  - about 4.3–5 MB brotli of wasm per web visitor;
  - no screen-reader editing in the browser;
  - egui API churn every 2–3 months.
- Risk table: §19.14. Open owner questions: §19.15.

**Implication for the review:** M2 already has internal boundaries that look like plugin seams (`EditorHost`,
`BoardView`, `Transport`). A plugin architecture could:
- **(a)** formalize those seams inside M2;
- **(b)** come after M2;
- **(c)** replace M2's plan.

Say which and why.

---

## 4. A sketch to evaluate (not a decision)

**Core** (always present):
- items tree, collections, access, users and tokens;
- revisions, trash and archive, attachments and blobs;
- search index, link graph;
- the sync transport;
- API routing, CLI framework, the plugin host.

**What a plugin could provide.** Item-type plugins provide most of this list; integration plugins provide a subset:
- **Server:**
  - a content type (`kind`), and how its content is stored;
  - validation;
  - text extraction for search, and outgoing links and embeds for the link graph;
  - export and import formats;
  - previews;
  - extra API routes or "actions";
  - background jobs;
  - settings and secrets.
- **Client:**
  - an editor view and a read-only view;
  - an HTML read view, for M2's `/read/` (§19.7);
  - an embed or card view for use inside other items (a sheet in a document, a GitHub issue on a board);
  - commands, menus and icons.
- **CLI/API:** typed or generic access to the content (`wikanvas item get --format …`).

**Plugin tiers** to weigh:

| Tier | How it ships | Trust | Who writes it |
|---|---|---|---|
| Built-in | Rust crates compiled into the binary, behind a trait and a registry (Cargo features) | full | the owner |
| Sandboxed | WebAssembly components loaded at runtime (server: wasmtime; client: see Q3) | capability-limited | anyone, later |

A likely path is built-in first, with the same interface later exposed through WIT to sandboxed plugins. Zed's
editor does it this way (§9). Check whether the built-in interface can be designed now so that it maps onto WIT later
without a rewrite.

---

## 5. Design questions to answer

**Q1. Delivery: built-in, sandboxed, or both?**
- Is a built-in-only plugin system worth having (G6), or does it only add indirection?
- If both, can one Rust trait serve as both the in-process interface and the WIT-generated one?

**Q2. Server-side sandbox**
- Can wasmtime components (WIT, `wasm32-wasip2`) carry what a server plugin needs:
  - reads and writes to its own storage through host functions, never raw SQLite;
  - calls into access checks, search indexing and links;
  - outbound HTTP for integrations;
  - access to secrets?
- What resource limits (fuel or epoch interruption, memory caps) are needed under G7: 2 CPUs and 1 GiB for everything?
- What does wasmtime add to binary size and build time under the build caps (§8)?
- wasmtime's licence is Apache-2.0 WITH LLVM-exception. Is that inside R6's list? Flag it for the owner if not.

**Q3. Client-side plugins in an egui wasm app (likely the hardest question)**
- M2 ships one wasm binary (`wikanvas-web`, `wasm32-unknown-unknown`). wasm has no dynamic linking. How could a
  plugin loaded at runtime draw UI into the same `egui::Context` in the browser?
- Options to evaluate:
  - **(a)** Client UI only for built-in plugins. Runtime plugins get server logic plus a **declarative UI** (forms,
    tables, lists, blocks described as data and drawn by the host).
  - **(b)** The plugin runs in a separate wasm instance and returns draw lists or a widget tree each frame. Weigh the
    cost, input handling, IME and focus.
  - **(c)** One instance per plugin with its own canvas or iframe. This contradicts D101, "no iframe".
  - **(d)** Natively (desktop), wasmtime inside the client. The browser would then need something different, for
    example jco, which is JavaScript and against G2.
- Whatever you recommend must work in both desktop and browser modes.

**Q4. Storage model for plugin content**
- Options:
  - **(a)** A **Yjs document per item** as the universal container. Plugins define their Y structure; yrs is already
    in the core. Sync, history, offline outbox, multi-user merge and revisions then come for free.
  - **(b)** Plugin-owned SQL tables through namespaced migrations.
  - **(c)** Blob or JSON snapshots, as boards do today.
  - **(d)** A mix.
- Boards today are snapshots, not Yjs. Can they stay a snapshot-type plugin, and what does the core then guarantee
  for them?
- How does `item_kinds` / `ItemKind` become an open registry without breaking `/api/v1` (G8: additive only, a closed
  enum in the spec today) and without migrating stored rows (G5)?

**Q5. Sync and collaboration**
- Live co-editing isn't built yet (pull protocol only; push via SSE is planned in M2 §19.4.6).
- Does a plugin architecture fix the sync model for every plugin, for example "plugins are Y docs"? Is that right for
  sheets, where cell-level merge is needed and a formula engine (IronCalc) keeps its own model?
- How do the Y structure and the engine's model stay consistent?

**Q6. Cross-cutting features plugins must join**

| Feature | Question for the review |
|---|---|
| Access | Where are the enforcement points? A plugin must not be able to leak an item or a title the viewer may not read, including inside embeds of other plugins' items (today's "redact cards" problem, generalized). Is this enforceable for sandboxed plugins? |
| Search | `item_text` |
| Links and backlinks | `item_links` |
| Revisions and restore | |
| Trash and archive | |
| Export and backup | Including the whole-workspace tar |
| Read view HTML | M2 §19.7 |
| Previews | |
| CLI/API | Including the agent harness (`docs/AGENTS.md`) |

**Q7. Embeds across plugins**
- Today's doc-in-board embed was costly (§3.2). Can a general "item X embedded in item Y" contract cover:
  - a sheet in a document;
  - a GitHub issue on a board;
  - a document card on a board, without per-pair code?

**Q8. Interfaces and versioning**
- How are the plugin interface (Rust trait and/or WIT) and stored plugin formats versioned?
- What happens to a workspace whose plugin is removed or downgraded?
- Can plugin data stay readable (export, read view) without the plugin?

**Q9. Integrations, the GitHub case**
- GitHub needs outbound HTTPS, OAuth or app tokens, secret storage and maybe webhooks (inbound, through Caddy). This
  conflicts with:
  - D29 (no outbound calls, no TLS in the server);
  - the `webpki-roots` ban (roots would have to come from the OS store, for example rustls-native-certs or
    rustls-platform-verifier: check licences).
- Is a separate integration process or sidecar better than linking TLS into the server?
- Where do secrets live (SQLite, encrypted? environment?), and who may use them under per-collection access?

**Q10. Migration from Wikanvas**
- Can documents and boards become the first two plugins **with no change to stored bytes** (G5)?
- Which existing code moves behind the interface? List the tables, routes, CLI commands and `wikanvas-core` modules.
- Can the live wiki stay up through every step? Each step must be a pull request with green CI that the owner can
  roll back.

**Q11. Size and performance**
- The browser download is already 4.3–5 MB brotli in M2. Can the client load plugin code lazily, for example several
  wasm modules, or does every built-in plugin ship in the one binary?
- What does that do to first load?

**Q12. Maintainability (G6)**
- Does a plugin boundary reduce the special cases (doc vs board branches in store, server, CLI, client), or move them
  into a wider interface that is harder to change?
- Point to where today's code would get simpler or more complex.

---

## 6. The first new plugins

### 6.1 Sheets

- Excel-like grid: cells, formulas, maybe several tabs. Multi-user editing expected eventually.
- Candidate engine: **IronCalc**. Rust, MIT OR Apache-2.0, compiles to wasm, Excel-compatible functions (sources say
  300–500+). Its README calls it a "work-in-progress" engine.
- To judge:
  - whether IronCalc can run on both server (recalculation, search text, export) and client (editing);
  - how its model maps onto a Y document for merges;
  - how to render a large grid in egui (virtualized).
- Alternative to study for design, not code: Grist (Apache-2.0, spreadsheet-database hybrid, Python formulas, custom
  widgets).

### 6.2 GitHub integration

The scope isn't decided (an owner question). Candidates:
- live embeds of issues, PRs and their CI status in documents and boards;
- an item linked to a repository file, kept in sync (pull, maybe push);
- a page of issues filtered by a query;
- webhooks for live updates.

Evaluate which of these fit the plugin model, and which need core features such as background jobs, outbound
network, secrets and inbound webhooks. A Rust GitHub client exists (octocrab; check its licence and TLS stack against
R6).

---

## 7. Prior art (web research 2026-10-30, partly unverified)

The closest products. None combines all four of: self-hosted, multi-user, plugin-first, Rust core.

| Product | Relevance | Plugins | Stack | Licence |
|---|---|---|---|---|
| AFFiNE | Docs + "edgeless" whiteboard + databases, self-hostable. Closest in features | None shipped. "Coming soon" in an Apr 2026 video, unverified | TypeScript (BlockSuite editor); Rust in OctoBase storage and the y-octo sync engine | MIT for most code; backend "AFFiNE EE" |
| AppFlowy | Notion-like: docs plus grid, board and calendar views | Few | Flutter + Rust core and server | AGPL-3.0 (ideas only) |
| Obsidian | The plugin model the owner has in mind: 4,000+ plugins and themes, a new Community directory (May 2026), paid plugins reportedly allowed | Yes, JS | Electron | Closed source |
| SilverBullet | Closest in spirit: built-in features are themselves "plugs"; Lua scripting; personal and Markdown-based | Yes | Go server, TypeScript client | MIT |
| Logseq | Plugin API, whiteboards; new database-backed version in beta | Yes, JS | Clojure | AGPL-3.0 (ideas only) |
| Trilium Notes | Note tree, scripting, Excalidraw-based canvas notes, self-hosted sync | Scripting | TypeScript | AGPL-3.0 (ideas only) |
| XWiki | Multi-user team wiki where apps, macros and server code are installable extensions (Extension Manager). Proof the model works for teams | Yes, Java | Java | LGPL-2.1 |
| Outline, Docmost | Close to what Wikanvas does today; no plugin systems | No | TypeScript | BSL 1.1, AGPL (ideas only) |
| Atomic Server | Rust: collaborative documents plus Airtable-like tables; alpha | No | Rust | Check |
| Grist | Sheets plus a database, custom widgets | Widgets | TypeScript + Python | Apache-2.0 |

Lessons to weigh:
- Obsidian's and Logseq's plugin APIs are JS with full trust, not sandboxed. Obsidian moved to reviewed distribution
  in 2026.
- SilverBullet shows "the product is its own plugins" at small scale.
- XWiki shows it for multi-user, with a heavy runtime.

---

## 8. Constraints for anyone running anything in this repository

From `CLAUDE.md`. Binding.
- Every cargo, rustc, node and pnpm command runs only through `scripts/dev.sh` (Docker, capped at 6 CPUs and 12 GB,
  one build at a time by lock). Never on the host, never with higher caps, never `sudo`.
- Never build inside or modify `vendor/inkboard`. Format only with `scripts/fmt.sh`.
- Licences as in G4. Reading AGPL or BSL projects for ideas is fine; copying their code is not.
- Wikanvas is live: changes reach `main` only through pull requests with green CI. Deploying to the owner's wiki needs
  the owner's go-ahead.
- Open owner questions stay open (ARCHITECTURE §16 item 25, §19.15). Don't decide them.

---

## 9. Rust building blocks (check versions and licences before relying on them)

| Need | Candidate | Notes |
|---|---|---|
| Server plugin sandbox | wasmtime + the WebAssembly component model (WIT, wit-bindgen) | Zed runs its extensions this way: Rust compiled to `wasm32-wasip2`, sandboxed, WIT interfaces versioned per release (`zed_extension_api`). Licence: Apache-2.0 WITH LLVM-exception (R6 check) |
| Ready-made plugin framework | Extism (Rust host SDK `extism`, plugin kit `extism-pdk`) | BSD-3-Clause; built on wasmtime; host functions, memory limits, host-controlled HTTP. Less typed than WIT |
| CRDT and sync | yrs 0.28 (already used) | Alternatives exist (Loro, Automerge), but documents are stored as Yjs: switching is a migration |
| Spreadsheet engine | IronCalc | MIT OR Apache-2.0; wasm; work in progress |
| UI | egui / eframe 0.37 (M2) | No dynamic loading in the browser build; see Q3 |
| GitHub API | octocrab | Check licence and TLS features against R6 and D29 |
| TLS roots without webpki-roots | rustls-native-certs or rustls-platform-verifier | Check licences |

---

## 10. Questions only the owner can answer (list them, don't decide them)

1. Built-in plugins only, or a public, sandboxed plugin API for others? If public: free only, or a directory?
2. Which GitHub features first (§6.2)?
3. PlugWiki and M2: formalize plugin seams inside M2, do M2 first, or re-plan M2 around plugins?
4. Must every plugin work in the browser **and** on the desktop, or may some be desktop-only or server-only?
5. May the server make outbound HTTPS calls (reverses D29), or do integrations run as a separate process?
6. The rename itself: the name PlugWiki, its domain, and when to rename. A rename touches about 6,000 occurrences in
   336 files, the CLI binary, `WIKANVAS_*` variables, images, slices and the live wiki's compose project (see D142).
