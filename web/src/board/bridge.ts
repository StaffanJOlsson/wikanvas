// The Quire frame API (docs/ARCHITECTURE.md §10.4): what the board frame page (`web/inkboard-frame/frame.js`, loaded
// from `/inkboard/{build}/frame.html`) installs as `window.quireBoardFrame`, and what the SPA's BoardFrameHost calls
// through `iframe.contentWindow.quireBoardFrame` (same origin, plain JS calls). Types only: frame.js imports them
// through JSDoc, so this file must not import anything.

export interface QuireBoardFrame { connect(events: FrameEvents): Promise<BoardFrameApi>; }  // window.quireBoardFrame
export interface FrameEvents {
  onProgress?(p: { loaded: number; total: number }): void;   // decoded wasm bytes / manifest raw size
  onReady?(itemId: string): void;
  onChange?(e: { itemId: string; revision: number }): void;
  onSave?(itemId: string): void;
  onError?(e: { itemId: string | null; kind: 'unsupported' | 'webgl' | 'download' | 'start' | 'load' | 'panic'; message: string }): void;
  onHostKey?(e: { key: string; mod: boolean; shift: boolean; alt: boolean }): void;
}
export interface OpenArgs { itemId: string; board: string | null; name: string; readOnly: boolean; theme: 'light' | 'dark' }
export interface BoardFrameApi {
  readonly engineVersion: string;     // inkboardEmbed.version + ".quire." + patchset, e.g. "0.2.0-beta.3+embed.quire.1"
  readonly formatVersion: number;
  open(a: OpenArgs): Promise<void>;   // first call mounts on the frame's canvas; later calls setBoard + setReadOnly + setName
  current(): string | null;           // itemId shown
  snapshot(): { itemId: string; board: string; revision: number };
  isDirty(): boolean;
  markSaved(revision: number): void;
  setTheme(t: 'light' | 'dark'): void;
  setReadOnly(b: boolean): void;
  setName(n: string): void;
  setActive(b: boolean): void;
  focus(): void;
  fitView(): void;
}

// Names for the inline shapes above (additive; they are exactly the shapes §10.4 spells out).

/** `FrameEvents.onError`'s argument. */
export type FrameError = Parameters<NonNullable<FrameEvents['onError']>>[0];
/** Every failure kind the frame reports. */
export type FrameErrorKind = FrameError['kind'];
/** `FrameEvents.onHostKey`'s argument: a key the frame forwards to the parent (Mod+K, Mod+., double Esc; §9.5). */
export type FrameHostKey = Parameters<NonNullable<FrameEvents['onHostKey']>>[0];
/** What `connect` rejects with, e.g. `{kind: 'unsupported', message}` from a frame without an engine. */
export type ConnectError = { kind: FrameErrorKind; message: string };

declare global {
  interface Window {
    /** Installed by the frame page's module script; absent until it ran (and on every other page). */
    quireBoardFrame?: QuireBoardFrame;
  }
}
