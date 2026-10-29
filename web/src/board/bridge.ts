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
  // M1.1, board document embeds (docs/ARCHITECTURE.md §18.4):
  /** The cards of the current page and the camera, after a frame in which any of it changed (at most one per frame). */
  onEmbedLayout?(e: EmbedLayout): void;
  /** The user asked to edit a card (double-click, Enter, or the context menu's "Edit text"): the host may open it.
   *  Never sent while the board is read-only. */
  onEmbedOpen?(e: EmbedOpenEvent): void;
  /** The engine ended the editing set by `setEmbedEditing` (a press in the frame, the frame took focus, the card left). */
  onEmbedEditEnd?(e: EmbedEditEndEvent): void;
  /** The frame took the keyboard focus during that editing without ending it (the middle button's pan: eframe focuses
   *  its canvas on every release over it); the host may give the focus back to the card. */
  onEmbedFocusBack?(e: EmbedFocusBackEvent): void;
  /** A sidebar document was dropped on the canvas (`DragItemType`); the host decides whether to insert it. */
  onDropItem?(e: DropItemEvent): void;
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
  // M1.1, board document embeds (§18.4). Optional: a frame without them (M1, the fake frame before E3) still types,
  // and the host offers embeds only when the connected frame has `insertEmbed`.
  /** Adds a card to the current page (one undo step, on the first visible unlocked layer) and selects it; its id, or
   *  null when the board is read-only. Throws while no board is open. */
  insertEmbed?(a: InsertEmbedArgs): string | null;
  /** The card being edited in the host (null: none). The engine keeps it selected, hides its handles, handles no key
   *  while it lasts, and ends the editing itself on a press in the frame or when the frame takes focus
   *  (`onEmbedEditEnd`). An id that is not a card on the current page's visible layers is ignored. */
  setEmbedEditing?(objectId: string | null): void;
  /** The live titles, by item id (already "Untitled" for a blank title). Replaces the previous map; the engine shows
   *  them as the cards' labels without an undo step or a revision, and the next save stores them. */
  setEmbedTitles?(titles: Readonly<Record<string, string>>): void;
  /** Sends `onEmbedLayout` after the next frame even when nothing changed. */
  requestEmbedLayout?(): void;
}

// ---------------------------------------------------------------------------------------------- board document embeds
// M1.1 (docs/ARCHITECTURE.md §18). Coordinates: world units are the board's own; a FramePoint or FrameRect is in CSS
// pixels of the frame's viewport (origin at the iframe's top left), so the host adds the iframe's client rect to get
// its own viewport coordinates. devicePixelRatio never enters (the engine reports CSS pixels).

/** The native drag type a sidebar document row carries (pragmatic-dnd `getInitialDataForExternal`, §18.6). */
export type DragItemType = 'application/x-quire-item';

/** The JSON under `DragItemType`. A reader ignores a value that does not parse, has another `v`, or is not a doc. */
export interface DraggedItem {
  v: 1;
  /** The item's UUID. */
  id: string;
  /** Only document rows carry the type in M1.1. */
  kind: 'doc';
  title: string;
}

/** A point in CSS pixels of the frame's viewport. */
export interface FramePoint {
  x: number;
  y: number;
}

/** A rectangle in CSS pixels of the frame's viewport. */
export interface FrameRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** One card on the current page: a `rect` box whose `kind.embed` holds `{item}` (§18.2). */
export interface EmbedBox {
  /** The Inkboard object id. */
  readonly objectId: string;
  /** `embed.item` as stored: normally an item UUID, but not checked by the engine (the host checks it). */
  readonly item: string;
  /** The frame in world units: top left, size, and the rotation in radians about its centre. */
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
  readonly angle: number;
  /** The label's plain text: the title snapshot the board stores (§18.7). */
  readonly label: string;
  readonly selected: boolean;
  /** The object or its layer is locked. */
  readonly locked: boolean;
}

/** Where the cards are: `screen = origin + world × zoom`, in frame-viewport CSS pixels. */
export interface EmbedLayout {
  /** The board. */
  readonly itemId: string;
  /** The current page's id. */
  readonly page: string;
  /** CSS pixels per world unit. */
  readonly zoom: number;
  readonly originX: number;
  readonly originY: number;
  /** The drawing area; cards are clipped to it. */
  readonly canvas: FrameRect;
  /** The engine's own interface over the drawing area (floating cards, menus, popups, tooltips, toasts, dialogs and
   *  their backdrop); cards are clipped out of these. */
  readonly occluders: readonly FrameRect[];
  /** The cards of the current page on visible layers, in drawing order (bottom first). */
  readonly embeds: readonly EmbedBox[];
  /** Every distinct `embed.item` on every page and layer, sorted. */
  readonly items: readonly string[];
  /** Every distinct `embed.item` of the cards on the engine's clipboard (cut or copied, still to be pasted, here or on
   *  the next board), sorted. Missing from an engine that does not report it. */
  readonly clipboard?: readonly string[];
  /** The card the engine holds as being edited (`setEmbedEditing`), or null. */
  readonly editing: string | null;
}

export interface EmbedOpenEvent {
  /** The board. */
  readonly itemId: string;
  readonly objectId: string;
  readonly item: string;
  /** Where the double-click was; null for Enter and the context menu. */
  readonly point: FramePoint | null;
}

export type EmbedEditEndReason = 'press' | 'focus' | 'gone';

export interface EmbedEditEndEvent {
  /** The board. */
  readonly itemId: string;
  readonly objectId: string;
  readonly reason: EmbedEditEndReason;
}

export interface EmbedFocusBackEvent {
  /** The board. */
  readonly itemId: string;
  /** The card being edited. */
  readonly objectId: string;
}

export interface DropItemEvent {
  /** The board. */
  readonly itemId: string;
  readonly item: DraggedItem;
  /** The drop point. */
  readonly point: FramePoint;
}

export interface InsertEmbedArgs {
  item: string;
  /** The title snapshot for the label ("Untitled" for a blank title: the host resolves it). */
  title: string;
  /** The centre of the new card: the centre of the view, or a frame-viewport point. */
  at: 'center' | FramePoint;
  /** World size; the engine's defaults are 480 × 360. */
  w?: number;
  h?: number;
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
