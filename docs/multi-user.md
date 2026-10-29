# Multi-user Wikanvas: research and a proposed path

**Status: a proposal; the owner answered questions 1, 2 and 4 on 2026-10-29** (§6). **Phases 1 and 2 are
implemented** (§5): the role check in every handler, and accounts with invite links (docs/ARCHITECTURE.md D148–D151). The accepted parts move into
docs/ARCHITECTURE.md as decisions when their phase starts, as M2 did (§19). It combines three studies made the
same day: how others handle several people editing at once, what Wikanvas has and lacks, and what upstream Inkboard
offers. Sources are numbered in brackets (§8).

## 1. Summary

- **Documents: keep Yjs, add a live channel.** Wikanvas already stores every document as Yjs updates and applies them
  with yrs on the server. Yjs is still the most used rich-text CRDT, MIT, with a Rust port (yrs) for the server and the
  planned egui client [13][19]. What is missing is the push: an open editor only sees other people's edits when it
  regains focus.
- **Boards: server-authoritative, not a CRDT.** Canvas tools converge on one model: a map of objects, the last write to
  an object or property wins, and a server orders the operations (Figma, tldraw; Excalidraw per element) [30][34][37].
  Inkboard's own session layer already works this way, so live boards should run that layer inside the Wikanvas
  server. That needs patches to Inkboard, ideally upstream.
- **Accounts and permissions come first.** The database already knows the roles owner, admin, member and viewer, but
  the role check is not wired into every route yet, and there is no way to invite anyone. Both must exist before a
  second person gets an account.
- **Inkboard: no upgrade now.** Its live sharing shipped in 0.2.0-beta.1 and our pin (beta.3) has all of it; upstream
  has nine small fixes since, no new tag. Upgrade at the next tag: our patches apply except two CHANGELOG hunks.
- **Order:** accounts and permissions → live documents → presence → history → live boards.

## 2. How others do it

**Google Docs: operational transformation with a central server** [1][2][3]. The document is a revision log. Each
client keeps at most one unacknowledged change in flight; the server transforms each change against everything
committed since the client's revision, numbers it and broadcasts it. The lineage is Xerox PARC's Jupiter (1995) and
Google Wave [4][5]. Limits: 100 people per file at once [6], offline editing only in Chrome or Edge [7], and weak
branching, since OT assumes one timeline kept by the server. Microsoft's Fluid goes further: its server only orders
operations without reading them, and the clients merge [8][9].

**What counts as a higher standard now: CRDTs and hybrids.** A CRDT merges edits in any order without a server
deciding, which gives offline editing and merging for free (Ink & Switch's "local-first" [11]).

| | Status | For Wikanvas |
|---|---|---|
| **Yjs / yrs** (MIT) | Yjs 13.6 stable, v14 at rc.29; yrs 0.28 [15][19] | What Wikanvas already stores. Rich ecosystem: ProseMirror binding, awareness, per-user undo. |
| **Automerge 3** (MIT, Rust core) | July 2025 rewrite cut memory over 10× [20] | Full history, Peritext-style formatting [21]. |
| **Loro** (MIT, Rust) | 1.x | Fugue text, Peritext-style rich text, movable tree, time travel [23][24]. The credible alternative. |
| **Eg-walker** (Diamond Types, ISC) | Research grade, EuroSys 2025 | CRDT/OT hybrid: much less memory, very fast merges of long branches [25]. |

Two known text anomalies: interleaving of concurrent insertions at one spot, which Yjs only shows in rare
multi-replica cases [26]; and overlapping formatting (two people bold overlapping ranges), which Yjs can get wrong
while Peritext-style marks in Automerge and Loro get right [27]. In a wiki both are rare and cosmetic. Neither is
worth migrating stored documents away from Yjs.

**Canvases.** Figma keeps `Map<ObjectID, Map<Property, Value>>` on a server that keeps the last write per property,
orders children with fractional indices and rejects parent cycles; clients apply their edits at once and ignore
incoming changes that conflict with their unacknowledged ones [30][31][33]. tldraw runs one authoritative room per
document and rebases client edits on the server's [34]. Excalidraw versions each element and relays end-to-end
encrypted messages [37][38]. The finer the conflict unit, the fewer lost edits: per property, one person can move a
shape while another recolours it.

**What every production system needs**, and the advice for Wikanvas:

- **Authorise every message,** not only the connection. Viewers receive updates; their writes are dropped. Drop live
  connections when a role changes.
- **Validate,** because CRDTs trust their clients and Yjs has no server middleware [42]: cap size and rate, accept only
  known roots, bind each Yjs client id to the first user who claims it, and check the document against the schema after
  applying an update. For boards, run the engine's own checks before committing.
- **Presence** (cursors, selections, who is here) lives in memory only. Clients write their own awareness state, so
  the server should stamp the signed-in identity into it [17].
- **Per-user undo:** undo only your own edits (Yjs UndoManager with local origins; Inkboard already keeps per-author
  histories) [18].
- **History:** an append-only update log with checkpoints [33], which Wikanvas already has (`doc_updates`, compaction).
- **Scaling:** one in-memory room per open document in one process is enough for a self-hosted wiki [32][34].

**Licences.** Usable as dependencies (all MIT): yjs, y-protocols, y-prosemirror, TipTap's collaboration packages and
`@tiptap/y-tiptap`, yrs, Loro, Automerge, the `fractional_index` crate, Hocuspocus (MIT, but a Node server). Not to be
copied from: `@liveblocks/server` and json-joy (AGPL), tldraw and its sync packages (source-available, not OSI) [36][50].

## 3. Where Wikanvas stands

**Accounts.** The schema has the four roles and allows exactly one owner and any number of other users. Sessions and
tokens belong to a user; token scopes (read ⊂ write ⊂ admin) are a separate layer on top of the role. Missing:

- **The role check is only partly wired.** `authz::require` runs in the document, board, file, workspace, export and
  admin routes, not in items, collections, lists, tree, search or tokens. Today the only account is the owner, who may
  do everything, so nothing is exposed; but it must be complete before a second account exists.
- **No user management:** no users, invites or members routes, and the admin commands act on the owner only.
- **Visibility is global:** the shared item filter ignores who is asking, title lookups resolve any item, and
  `config.multi_user` is hard-coded false. The tables `collection_members`, `item_permissions`, `invites`, `groups`
  and `shares` are named in ARCHITECTURE §15 but not created.
- **Client caches are per item, not per user:** the offline outbox and the unsaved-board snapshots would replay one
  person's edits under the next person's session.

**Documents.** Stored as an append-only Yjs update log with a user per update, compacted into a checkpoint. The web
editor already merges concurrent edits correctly (two tabs converge) and has an offline outbox with retries, but there
is no push channel, no room, no presence and no live title. Every save reloads and re-walks the document inside
SQLite's single writer, which becomes the bottleneck when several people type at once. The server records Yjs client
ids with `INSERT OR IGNORE`, so a client could claim another user's id.

**Boards.** Saved whole with `If-Match`; two editors get "Load latest" or "Keep mine as a copy", never a merge. Cards
already edit their documents through the document path, so cards become live with live documents.

**The M2 plan (§19).** It keeps live push out of M2's scope and already plans the transport for later: one
server-sent-event stream per client plus POST writes (§19.4.6, D118). It also says "no schema change in M2" (D143) so
an M1 image can open the database after a rollback. Accounts and permissions need a schema change, so either they land
before M2 starts, or D143 is amended.

## 4. Inkboard

- **What changed upstream** since our pin (41e0295, v0.2.0-beta.3) to main (d097526): nine fixes (pasting from Excel,
  shadows on new objects, arrowheads on short connectors, PDF shadows). No new tag, same licence, dependencies,
  wasm-bindgen 0.2.129 and file format.
- **How its collaboration works:** `inkboard-session` (pure Rust) has a `Sequencer` that orders edits on the host's
  copy; the later edit to an object replaces the whole object and the other side is told it was superseded. Guests
  apply edits at once and replay them on confirmation; undo is per author; presence covers cursors, selections,
  viewports, follow and laser; roles are Edit and View. `inkboard-net` carries it over iroh, which needs a relay and a
  desktop host: a browser can only join, nothing is stored on a server, and identity is a typed name. Wikanvas compiles
  that transport out (D30) and nothing of the session works in our browser build.
- **As a basis for Wikanvas:** the protocol layer fits (server-authoritative, keeps the native board format, its
  sequence number maps onto board revisions); the iroh topology does not. Running it in `wikanvas-server` needs patches
  that do not exist yet: resuming from a stored sequence number, a headless host taken out of the desktop app, a
  browser WebSocket connection and embed API calls, a trusted-server mode, and saving during a session. Proposing a
  transport-agnostic headless host upstream would shrink that series. Conflicts are per object today; per property
  would lose fewer edits.
- **Upgrade:** wait for the next tag. Cost then: resolve the two CHANGELOG hunks in patches 0004 and 0006, bump the
  patchset and the pins, rerun `inkboard.sh check`, `build` and the frame tests.

## 5. Proposed path

| Phase | What | Size | Notes |
|---|---|---|---|
| 1 | **Role check everywhere**: `authz::require` in every handler; purge by role (admin); a per-role test matrix | S–M | No change for the owner. Can start at once. |
| 2 | **Accounts** (implemented, D148–D151): migration 0002 (`invites`), `GET`/`PATCH /users`, invite create, list and revoke, a public invite page and accept, disable and enable (deleting an account, with its content kept, is left for later), `admin list-users`/`create-user`/`set-role`/`disable-user`/`enable-user` and `--email` on the token commands, `wikanvas user` and `wikanvas invite`, Settings → Members, `multi_user` from the count of enabled users | M | A pre-migration backup on the live wiki; D143 amended: M2's baseline is `user_version` 2 (§6 Q1). Until phase 3, every member and viewer sees every collection. |
| 3 | **Permissions**: workspace roles plus per-collection membership (read, write, manage); the item filter, search, tree ETag, title lookups, backlinks, attachments and board card labels all filtered by viewer | L | Leaks through any missed query are the risk: test every list per role. |
| 4 | **Per-user details**: author names in the API, "Alex updated 5 minutes ago", drafts private to their author (D23), the client caches keyed by user | M | |
| 5 | **Live documents** (in M2, Q4): a room per open document holding its yrs doc, push of accepted updates (including API writes) over one event stream per client, authorisation per message, client ids bound to users, the search-text walk moved off the hot path | L | Built with M2's client. |
| 6 | **Presence**: cursors, selections and who is here, with the identity stamped by the server; the live title | M | |
| 7 | **History**: named versions from checkpoints, attribution per update, restore as a new change | M | |
| 8 | **Live boards**: Inkboard's sequencer in the server, one room per board, presence and engine checks; the `If-Match` path stays for the API | L–XL | Needs the Inkboard patches of §4. |

## 6. Questions for the owner

1. **Order against M2. Answered: before M2.** Phases 1–4 land first, as migration 0002, so M2's "no schema change"
   baseline (D143) starts after them.
2. **Who may see what. Answered: workspace roles plus membership per collection** (read, write, manage); per-page
   sharing later.
3. **How people get accounts. Answered: invite links** made by the owner or an admin (no email server needed), no
   open sign-up (phase 2, D148).
4. **Live documents in the current web client, or only in M2. Answered: wait for M2.** Live documents and presence
   (phases 5 and 6) are built in M2's client and data layer, not in the React client M2 replaces; push over one
   server-sent-event stream per client plus the existing POST writes, as §19.4.6 plans.
5. **Live boards.** Open; default: through Inkboard's session layer in the server, after live documents, proposing a
   headless host to Inkboard's author first.
6. **Inkboard upgrade.** Open; default: at the next upstream tag.

## 7. The resulting order

Phases 1–4 (role check everywhere, accounts and invites, per-collection permissions, per-user details) now, on the
server and in the current web client; then M2; live documents and presence inside M2; history and live boards after.
Phases 1 and 2 are done; phase 3 (per-collection permissions) is next. Until it lands, every member and viewer sees
every collection, and members can change them: invite only people who may.

## 8. Sources

Primary sources, checked 2026-10-29; the full notes with 53 sources are in the session's research files.

1–3. Google Drive Blog, "What's different about the new Google Docs" (three posts), Sep 2010.
https://drive.googleblog.com/2010/09/whats-different-about-new-google-docs.html
4. D. Wang, A. Mah, S. Lassen, "Google Wave Operational Transformation", 2010.
5. Nichols et al., "High-latency, low-bandwidth windowing in the Jupiter collaboration system", UIST 1995.
6. Google Workspace Help, sharing limits. https://support.google.com/a/users/answer/9305685
7. Google Docs Help, offline. https://support.google.com/docs/answer/6388102
8–9. Fluid Framework architecture; "Fluid Framework 2 is now production ready", Jun 2024.
https://fluidframework.com/docs/concepts/architecture
11. Kleppmann et al., "Local-first software", Ink & Switch, 2019. https://www.inkandswitch.com/local-first/
13, 15. Yjs README and releases. https://github.com/yjs/yjs
17–18. Yjs docs, Awareness and UndoManager. https://docs.yjs.dev
19. yrs. https://github.com/y-crdt/y-crdt
20–21. Automerge 3.0 (Jul 2025) and 2.2 rich text (Apr 2024). https://automerge.org/blog/
23–24. Loro. https://github.com/loro-dev/loro
25. Gentle & Kleppmann, "Collaborative Text Editing with Eg-walker", EuroSys 2025. https://arxiv.org/abs/2409.14252
26. Weidner & Kleppmann, "The Art of the Fugue", IEEE TPDS 2025. https://arxiv.org/abs/2305.00583
27. Litt et al., "Peritext: A CRDT for Rich-Text Collaboration", 2021. https://www.inkandswitch.com/peritext/
30–31, 33. Figma: "How Figma's multiplayer technology works" (2019), "Realtime editing of ordered sequences" (2017),
"Making multiplayer more reliable" (2022). https://www.figma.com/blog/how-figmas-multiplayer-technology-works/
32. Figma, "Rust in production at Figma", 2018.
34, 36. tldraw sync docs and licence. https://tldraw.dev/docs/sync
37–38. Excalidraw, P2P collaboration and end-to-end encryption, 2020. https://plus.excalidraw.com/blog
42. Yjs forum, "Validation, security and middleware". https://discuss.yjs.dev/t/validation-security-and-middleware/2229
50. Liveblocks, sync engine licence, Feb 2026. https://liveblocks.io/blog
Inkboard: https://github.com/VictorLetirand/inkboard (`docs/SHARING.md`, `crates/inkboard-session`), at d097526.
