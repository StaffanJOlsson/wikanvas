// Types of `inkboard.js`, the wasm-bindgen `--target web` module of Inkboard built with the `embed` feature
// (docs/ARCHITECTURE.md §10.3, §10.5). scripts/inkboard.sh runs wasm-bindgen with --no-typescript and writes
// inkboard.js next to frame.js in the dist, so frame.js imports './inkboard.js' and TypeScript reads this file for it.
// It is never copied into the dist (only *.html, *.js and *.css are).

/** What `init` accepts, as wasm-bindgen's web target declares it. */
export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;
export type SyncInitInput = BufferSource | WebAssembly.Module;

/** The instantiated wasm exports. Quire uses none of them directly: the engine API is `globalThis.inkboardEmbed`. */
export interface InitOutput {
  readonly memory: WebAssembly.Memory;
}

/**
 * Instantiates the module and runs the wasm `main`, which installs `globalThis.inkboardEmbed`. frame.js passes
 * `{module_or_path: new Response(countingStream, {headers: {'Content-Type': 'application/wasm'}})}` (§10.4).
 */
export default function init(
  module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>,
): Promise<InitOutput>;
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/** The Inkboard JS API (§10.3), installed on `globalThis` by the wasm `main`. */
export interface InkboardEmbed {
  readonly version: string;            // CARGO_PKG_VERSION + "+embed", e.g. "0.2.0-beta.3+embed"
  readonly formatVersion: number;      // inkboard_core::serial::VERSION (2)
  mount(canvas: HTMLCanvasElement, opts: MountOptions): Promise<InkboardHandle>;
  // rejects {kind:'webgl'|'load'|'start', message}; 'start' with "already mounted" while another handle is live
}

export interface MountOptions {
  board?: string | null;               // native envelope JSON; null → new empty board
  name?: string;                       // doc.name (export file names)
  theme?: 'light' | 'dark' | 'board';  // chrome only; default 'board' (today's behaviour)
  lang?: 'en' | 'fr';                  // i18n::set after App::new; default: browser locale (today's behaviour)
  readOnly?: boolean;
  onReady?(): void;
  onChange?(e: { revision: number }): void;
  onSave?(): void;                     // Ctrl/Cmd+S or menu Save
  onError?(e: { kind: 'panic'; message: string }): void;
  // Patch 0005, embedded documents (docs/ARCHITECTURE.md §18.3). Called after the frame, like the others.
  onEmbedLayout?(e: EngineEmbedLayout): void;   // at most once per frame, when anything in it changed
  onEmbedOpen?(e: EngineEmbedOpen): void;       // instead of editing a card's label; never while read-only
  onEmbedEditEnd?(e: EngineEmbedEditEnd): void; // the editing set by setEmbedEditing ended in the engine
}

export interface InkboardHandle {
  snapshot(): { board: string; revision: number };  // synchronous; includes text being typed; works in hidden tabs
  setBoard(json: string | null, name?: string): void; // resets undo and transient editor state; null: a new empty board; throws {kind:'load', message}
  revision(): number;
  isDirty(): boolean;
  markSaved(revision: number): void;
  setTheme(t: 'light' | 'dark' | 'board'): void;
  setReadOnly(b: boolean): void;
  setName(name: string): void;                      // assigns ed.doc.name directly: no undo entry, no revision bump
  setActive(b: boolean): void;                      // false: no periodic repaint (hidden host); true: repaint now
  focus(): void;
  fitView(): void;
  destroy(): void;                                  // runner.destroy(), clear_ctx(); handle becomes inert; mount allowed again
  // Patch 0005, embedded documents (§18.3); absent from an engine built without it.
  insertEmbed?(a: EngineInsertEmbed): string | null; // one undo step, selected; null when read-only; throws {kind:'start'} when inert
  setEmbedEditing?(id: string | null): void;        // while set: no handles, no shortcut; ended by a press or focus
  setEmbedTitles?(titles: Record<string, string>): void; // replaces the map; labels change in place (no undo, no revision)
  requestEmbedLayout?(): void;                      // onEmbedLayout after the next frame even when unchanged
}
// An inert handle (after destroy, or once the engine stopped) throws {kind:'start'} from snapshot() and setBoard(), returns 0
// from revision() and false from isDirty(), and ignores the other calls.

// Patch 0005 (docs/ARCHITECTURE.md §18.3). Engine coordinates are CSS pixels of the canvas element (top left 0, 0):
// egui points times `zoom_factor`. frame.js adds the canvas's offset in the frame to make them frame-viewport points
// (bridge.ts FramePoint, FrameRect). World units are the board's own.

/** A rectangle in CSS pixels of the canvas element. */
export interface CanvasRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** A card: a `rect` box with `kind.embed = {item}`, on a visible layer of the current page. */
export interface EngineEmbed {
  id: string;                          // the object id
  item: string;                        // embed.item as stored
  x: number;                           // the frame in world units
  y: number;
  w: number;
  h: number;
  angle: number;                       // radians, about the frame's centre
  label: string;                       // the label's plain text (the title snapshot)
  selected: boolean;
  locked: boolean;                     // the object or its layer
}

export interface EngineEmbedLayout {
  page: string;                        // the current page's id
  zoom: number;                        // CSS px per world unit: view.zoom × zoom_factor
  originX: number;                     // canvas CSS px of world (0, 0)
  originY: number;
  canvas: CanvasRect;                  // the drawing area (the central panel)
  occluders: CanvasRect[];             // every visible egui layer above Order::Background
  embeds: EngineEmbed[];               // drawing order, bottom first
  items: string[];                     // distinct embed.item values of every page and layer, sorted
  editing: string | null;              // the id set by setEmbedEditing, while it lasts
}

export interface EngineEmbedOpen {
  id: string;
  item: string;
  point: { x: number; y: number } | null; // canvas CSS px of the double-click; null for Enter or the menu
}

export interface EngineEmbedEditEnd {
  id: string;
  reason: 'press' | 'focus' | 'gone';
}

export interface EngineInsertEmbed {
  item: string;
  title: string;
  at: 'center' | { x: number; y: number }; // the new card's centre: the view's centre, or a canvas CSS px point
  w?: number;                          // world units; default 480
  h?: number;                          // default 360
}

/** What `mount` rejects with, and what `setBoard` throws (`kind: 'load'`). */
export interface InkboardError {
  kind: 'webgl' | 'load' | 'start';
  message: string;
}

declare global {
  /** Set by the wasm `main` once `init` has run; undefined before. */
  var inkboardEmbed: InkboardEmbed | undefined;
}
