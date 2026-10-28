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
}
// An inert handle (after destroy, or once the engine stopped) throws {kind:'start'} from snapshot() and setBoard(), returns 0
// from revision() and false from isDirty(), and ignores the other calls.

/** What `mount` rejects with, and what `setBoard` throws (`kind: 'load'`). */
export interface InkboardError {
  kind: 'webgl' | 'load' | 'start';
  message: string;
}

declare global {
  /** Set by the wasm `main` once `init` has run; undefined before. */
  var inkboardEmbed: InkboardEmbed | undefined;
}
