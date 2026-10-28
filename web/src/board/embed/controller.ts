// The board embeds controller (docs/ARCHITECTURE.md §18.5–§18.7): the host's side of the cards, outside React.
// - It is the host's `EmbedSink`: each `onEmbedLayout` of the current board is applied to the overlay's DOM at once,
//   through refs (the world transform, each drawn card's box, the clip path), never through React state. The store
//   below changes, and React renders, only when the drawn cards change (ids, items, labels, selection, which one is
//   editing), so a pan, a zoom or a drag touches no component.
// - Editing: one card at a time (D65, D66). `onEmbedOpen` for a live, unarchived document of a writable board:
//   `setEmbedEditing(id)`, the card focused at once, then a `DocSession` (after the previous one's last flush); the
//   card renders `QuireEditor` on it. Editing ends on Esc (then `frame.focus()`), `onEmbedEditEnd`, a pointerdown in
//   the parent outside the card and the popup root, a later layout without the card or no longer editing it, the
//   board turning read-only, the target trashed or archived, the board released, a frame reset. Ending is idempotent:
//   the editor's last JSON goes into `['docJson', id]`, the session flushes and is destroyed, the key is invalidated.
// - Adding: `insertCard` (the "Text doc" button, the picker and both drop paths end there) and `dropItem`.
// - `popupRoot`: the editor's popups (`.board-embed-popups`) in `document.body`, moved into the frame layer while it
//   is the fullscreen element (Focus).
import type { QueryClient } from '@tanstack/react-query';
import type { Editor } from '@tiptap/react';
import { create } from 'zustand';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import { docJsonKey, type CachedDocJson } from '../../api/queries/docs';
import { fetchItem, itemKey } from '../../api/queries/items';
import type { DocSession } from '../../editor/docSession';
import { toast } from '../../state/toasts';
import type {
  BoardFrameApi,
  DraggedItem,
  DropItemEvent,
  EmbedBox,
  EmbedEditEndEvent,
  EmbedLayout,
  EmbedOpenEvent,
  FramePoint,
} from '../bridge';
import { setEmbedSink, useHostStore, type EmbedSink } from '../host';
import { isItemId } from './dragItem';
import { caretFor, cardBox, chooseCards, clipPath, viewportToFrame } from './layout';
import { cachedItem, snapshotTitle, TitleSync } from './titles';

/** A card the overlay draws. */
export interface DrawnCard {
  readonly objectId: string;
  readonly item: string;
  /** The engine's title snapshot (shown while the target loads, or struck through when it is missing). */
  readonly label: string;
  readonly selected: boolean;
}

/** The card being edited, as the overlay renders it. */
export interface EditingCard {
  readonly token: number;
  readonly objectId: string;
  readonly item: string;
  /** Null while the session opens (the card keeps its static viewer meanwhile). */
  readonly session: DocSession | null;
}

/** The board the overlay serves: opened in the frame by the board page. */
export interface EmbedBoard {
  readonly itemId: string;
  readonly readOnly: boolean;
}

export interface EmbedStore {
  board: EmbedBoard | null;
  cards: readonly DrawnCard[];
  editing: EditingCard | null;
}

export const useEmbedStore = create<EmbedStore>(() => ({ board: null, cards: [], editing: null }));

export const OPEN_FAILED = 'Could not open the page';
export const INSERT_FAILED = 'Could not add the page to the board';

/** How `insertCard` places a card. */
export interface InsertOptions {
  /** Start editing the new card (the "Text doc" button); otherwise the frame gets focus. */
  edit?: boolean;
  /** Place it only while this board is the open one (the board the action was started on). */
  board?: string;
}

/** A target a card may be edited for: a live, unarchived document. */
export function isEditableTarget(d: ItemDetail): boolean {
  return d.item.kind === 'doc' && d.trashed_at === null && d.archived_at === null;
}

interface Editing {
  token: number;
  objectId: string;
  item: string;
  point: FramePoint | null;
  /** The layout when the open was reported (the double-click point is in its camera). */
  openLayout: EmbedLayout | null;
  /** A layout held `editing === objectId`. */
  sawEditing: boolean;
  session: DocSession | null;
}

function frameApi(): BoardFrameApi | null {
  return useHostStore.getState().frame;
}

function sameCards(a: readonly DrawnCard[], b: readonly DrawnCard[]): boolean {
  return (
    a.length === b.length &&
    a.every((c, i) => {
      const d = b[i];
      return c.objectId === d.objectId && c.item === d.item && c.label === d.label && c.selected === d.selected;
    })
  );
}

function applyBox(el: HTMLElement, e: EmbedBox): void {
  const b = cardBox(e);
  el.style.left = `${b.left}px`;
  el.style.top = `${b.top}px`;
  el.style.width = `${b.width}px`;
  el.style.height = `${b.height}px`;
  el.style.transform = e.angle !== 0 ? `rotate(${e.angle}rad)` : '';
}

let instance: EmbedController | null = null;

/** The mounted overlay's controller (null without a frame host). */
export function embedController(): EmbedController | null {
  return instance;
}

export class EmbedController implements EmbedSink {
  readonly popupRoot: HTMLDivElement;
  private readonly qc: QueryClient;
  private readonly titles: TitleSync;
  private last: EmbedLayout | null = null;
  private drawn = new Set<string>();
  private readonly cardEls = new Map<string, HTMLElement>();
  private readonly titleEls = new Map<string, HTMLElement>();
  private overlay: HTMLElement | null = null;
  private world: HTMLElement | null = null;
  private ed: Editing | null = null;
  private seq = 0;
  /** The last ended session's flush (the next session waits for it). */
  private closing: Promise<void> = Promise.resolve();
  private editor: Editor | null = null;
  /** Refetch the targets' content and details at the first layout after an open. */
  private refresh = false;
  private focusOnMount: string | null = null;
  private attached = false;

  constructor(qc: QueryClient) {
    this.qc = qc;
    this.titles = new TitleSync({ qc, frame: frameApi });
    this.popupRoot = document.createElement('div');
    this.popupRoot.className = 'board-embed-popups';
  }

  /** Becomes the host's sink and the module's controller; puts the popup root into the document. */
  attach(): void {
    if (this.attached) return;
    this.attached = true;
    instance = this;
    setEmbedSink(this);
    document.body.appendChild(this.popupRoot);
    document.addEventListener('fullscreenchange', this.onFullscreen);
    this.titles.start();
  }

  dispose(): void {
    if (!this.attached) return;
    this.end({ tellEngine: false });
    this.attached = false;
    if (instance === this) {
      instance = null;
      setEmbedSink(null);
      useEmbedStore.setState({ board: null, cards: [], editing: null });
    }
    this.popupRoot.remove();
    document.removeEventListener('fullscreenchange', this.onFullscreen);
    this.titles.dispose();
    this.last = null;
    this.drawn = new Set();
  }

  /** The layout last applied. */
  layoutNow(): EmbedLayout | null {
    return this.last;
  }

  // ------------------------------------------------------------------------------------------- DOM registration

  setOverlay = (el: HTMLElement | null): void => {
    this.overlay = el;
    this.applyDom();
  };

  setWorld = (el: HTMLElement | null): void => {
    this.world = el;
    this.applyDom();
  };

  /** A card's box element (null: it unmounted). Its box is applied at once from the last layout. */
  registerCard(objectId: string, el: HTMLElement | null): void {
    if (!el) {
      this.cardEls.delete(objectId);
      return;
    }
    this.cardEls.set(objectId, el);
    const e = this.last?.embeds.find((x) => x.objectId === objectId);
    if (e) applyBox(el, e);
    if (this.focusOnMount === objectId) {
      this.focusOnMount = null;
      el.removeAttribute('aria-hidden');
      el.tabIndex = -1;
      el.focus({ preventScroll: true });
    }
  }

  /** The editing card's title input. */
  registerTitleInput(objectId: string, el: HTMLElement | null): void {
    if (el) this.titleEls.set(objectId, el);
    else this.titleEls.delete(objectId);
  }

  /** Where Backspace at the start of the card's empty body goes. */
  titleInput(objectId: string): HTMLElement | null {
    return this.titleEls.get(objectId) ?? null;
  }

  private onFullscreen = (): void => {
    const layer = useHostStore.getState().layer;
    if (layer && document.fullscreenElement === layer) {
      if (this.popupRoot.parentNode !== layer) layer.appendChild(this.popupRoot);
    } else if (this.popupRoot.parentNode !== document.body) {
      document.body.appendChild(this.popupRoot);
    }
  };

  // ------------------------------------------------------------------------------------------- the sink

  layout(l: EmbedLayout): void {
    this.last = l;
    this.titles.setItems(l.items);
    if (this.refresh) {
      this.refresh = false;
      for (const id of l.items) {
        if (!isItemId(id)) continue;
        void this.qc.invalidateQueries({ queryKey: docJsonKey(id) });
        if (cachedItem(this.qc, id)) void this.qc.invalidateQueries({ queryKey: itemKey(id) });
      }
    }
    this.checkEditing(l);
    this.publish();
    this.applyDom();
  }

  open(e: EmbedOpenEvent): void {
    const b = useEmbedStore.getState().board;
    if (!b || b.itemId !== e.itemId || b.readOnly) return;
    if (this.ed?.objectId === e.objectId) return;
    if (!isItemId(e.item)) return;
    const d = cachedItem(this.qc, e.item);
    if (d ? !isEditableTarget(d) : this.qc.getQueryState(itemKey(e.item))?.status === 'error') return;
    this.start(e.objectId, e.item, e.point, this.last);
  }

  editEnd(e: EmbedEditEndEvent): void {
    if (this.ed?.objectId === e.objectId) this.end({ tellEngine: false });
  }

  drop(e: DropItemEvent): void {
    this.dropItem(e.item, e.point);
  }

  reset(): void {
    this.end({ tellEngine: false });
    this.last = null;
    this.titles.reset();
    this.publish();
  }

  opened(a: { itemId: string; readOnly: boolean }): void {
    this.end({ tellEngine: false });
    useEmbedStore.setState({ board: { itemId: a.itemId, readOnly: a.readOnly } });
    this.refresh = true;
    this.titles.opened();
  }

  // ------------------------------------------------------------------------------------------- board state

  /** The board page's read-only state changed (trashed, archived, restored). */
  setReadOnly(itemId: string, readOnly: boolean): void {
    const b = useEmbedStore.getState().board;
    if (!b || b.itemId !== itemId) return;
    if (b.readOnly !== readOnly) useEmbedStore.setState({ board: { itemId, readOnly } });
    if (readOnly) this.end({ tellEngine: true });
  }

  /** The board page left `itemId` (route change, board switch). */
  release(itemId: string): void {
    const b = useEmbedStore.getState().board;
    if (b && b.itemId !== itemId) return;
    this.end({ tellEngine: true });
    if (b) useEmbedStore.setState({ board: null });
  }

  /** The overlay was hidden (off board routes, paused, failed). */
  hidden(): void {
    this.end({ tellEngine: true });
  }

  // ------------------------------------------------------------------------------------------- layout

  private publish(): void {
    const l = this.last;
    let cards: DrawnCard[] = [];
    if (l) {
      const ids = new Set(chooseCards(l, null, this.ed?.objectId ?? null, this.drawn));
      this.drawn = ids;
      cards = l.embeds
        .filter((e) => ids.has(e.objectId))
        .map((e) => ({ objectId: e.objectId, item: e.item, label: e.label, selected: e.selected }));
    } else {
      this.drawn = new Set();
    }
    const ed = this.ed;
    const editing: EditingCard | null = ed
      ? { token: ed.token, objectId: ed.objectId, item: ed.item, session: ed.session }
      : null;
    const st = useEmbedStore.getState();
    const patch: Partial<EmbedStore> = {};
    if (!sameCards(st.cards, cards)) patch.cards = cards;
    const prev = st.editing;
    if (prev?.token !== editing?.token || prev?.session !== editing?.session) patch.editing = editing;
    if (patch.cards || 'editing' in patch) useEmbedStore.setState(patch);
  }

  private applyDom(): void {
    const l = this.last;
    if (!l) return;
    if (this.world) {
      this.world.style.transform = `translate(${l.originX}px, ${l.originY}px) scale(${l.zoom})`;
    }
    for (const e of l.embeds) {
      const el = this.cardEls.get(e.objectId);
      if (el) applyBox(el, e);
    }
    this.overlay?.style.setProperty('clip-path', clipPath(l.canvas, l.occluders));
  }

  // ------------------------------------------------------------------------------------------- editing

  /** The card being edited (object id), or null. */
  editingId(): string | null {
    return this.ed?.objectId ?? null;
  }

  private checkEditing(l: EmbedLayout): void {
    const ed = this.ed;
    if (!ed) return;
    // Any layout received after setEmbedEditing must hold the card (a hidden layer, a page change, a delete).
    if (!l.embeds.some((e) => e.objectId === ed.objectId)) {
      this.end({ tellEngine: true });
      return;
    }
    // The first layout after onEmbedOpen was queued before setEmbedEditing and says `editing: null`; only a layout
    // after one that held the editing ends it.
    if (l.editing === ed.objectId) ed.sawEditing = true;
    else if (ed.sawEditing) this.end({ tellEngine: false });
  }

  /** Starts editing card `objectId` of target `item` (`point`: the double-click, frame px of `openLayout`). */
  start(objectId: string, item: string, point: FramePoint | null, openLayout: EmbedLayout | null): void {
    const frame = frameApi();
    if (!frame?.setEmbedEditing) return;
    if (this.ed) this.end({ tellEngine: false });
    const ed: Editing = { token: ++this.seq, objectId, item, point, openLayout, sawEditing: false, session: null };
    try {
      frame.setEmbedEditing(objectId);
    } catch (e) {
      console.warn('setEmbedEditing failed', e);
      return;
    }
    this.ed = ed;
    document.addEventListener('pointerdown', this.onPointerDown, true);
    this.publish();
    // Keys typed while the document loads must not reach the engine, and an Esc already cancels.
    // React renders the editing card's attributes soon; focus moves now (and a static card is aria-hidden).
    const el = this.cardEls.get(objectId);
    if (el) {
      el.removeAttribute('aria-hidden');
      el.tabIndex = -1;
      el.focus({ preventScroll: true });
    } else {
      this.focusOnMount = objectId;
    }
    void this.openSession(ed);
  }

  private async validTarget(item: string): Promise<boolean> {
    const cached = cachedItem(this.qc, item);
    if (cached) return isEditableTarget(cached);
    try {
      const d = await this.qc.fetchQuery({ queryKey: itemKey(item), queryFn: ({ signal }) => fetchItem(item, signal) });
      return isEditableTarget(d);
    } catch {
      return false;
    }
  }

  private async openSession(ed: Editing): Promise<void> {
    const valid = await this.validTarget(ed.item);
    if (this.ed !== ed) return;
    if (!valid) {
      this.end({ tellEngine: true, focusFrame: true });
      return;
    }
    await this.closing;
    if (this.ed !== ed) return;
    let session: DocSession;
    try {
      // Loaded with the editor's chunk, not with the shell (the overlay is part of every page).
      const { DocSession: Session } = await import('../../editor/docSession');
      if (this.ed !== ed) return;
      session = await Session.open(ed.item);
    } catch (e) {
      console.warn('opening the embedded page failed', e);
      if (this.ed === ed) {
        toast({ message: OPEN_FAILED, kind: 'error' });
        this.end({ tellEngine: true });
      }
      return;
    }
    if (this.ed !== ed) {
      void session.flush().catch(() => undefined).finally(() => session.destroy());
      return;
    }
    ed.session = session;
    this.publish();
  }

  /** The editing card's editor exists (or went: null). Moves focus and the caret into it. */
  editorReady(objectId: string, editor: Editor | null): void {
    const ed = this.ed;
    if (!editor) {
      if (ed?.objectId === objectId) this.editor = null;
      return;
    }
    if (!ed || ed.objectId !== objectId || !ed.session) return;
    this.editor = editor;
    const iframe = useHostStore.getState().iframe;
    const r = iframe ? iframe.getBoundingClientRect() : { left: 0, top: 0 };
    const caret = caretFor(ed.point, ed.openLayout, this.last, objectId, r);
    if (caret.kind === 'header') {
      const title = this.titleEls.get(objectId);
      if (title) {
        title.focus({ preventScroll: true });
        return;
      }
    }
    if (caret.kind === 'body') {
      editor.view.focus();
      const hit = editor.view.posAtCoords({ left: caret.left, top: caret.top });
      if (hit) {
        editor.commands.setTextSelection(hit.pos);
        return;
      }
    }
    editor.commands.focus('end');
  }

  /** Esc in the editing card, unconsumed. */
  escape(objectId: string): void {
    if (this.ed?.objectId === objectId) this.end({ tellEngine: true, focusFrame: true });
  }

  /** The target of the editing card became trashed, archived or missing. */
  targetGone(objectId: string): void {
    if (this.ed?.objectId === objectId) this.end({ tellEngine: true });
  }

  private onPointerDown = (ev: Event): void => {
    const ed = this.ed;
    if (!ed) return;
    const t = ev.target;
    if (t instanceof Node) {
      const card = this.cardEls.get(ed.objectId);
      if (card?.contains(t) || this.popupRoot.contains(t)) return;
    }
    this.end({ tellEngine: true });
  };

  /** Ends editing (idempotent). `tellEngine`: `setEmbedEditing(null)`; `focusFrame`: then `frame.focus()`. */
  end(opts: { tellEngine: boolean; focusFrame?: boolean }): void {
    const ed = this.ed;
    if (!ed) return;
    this.ed = null;
    this.focusOnMount = null;
    document.removeEventListener('pointerdown', this.onPointerDown, true);
    // What `start` set by hand, in case React never rendered the editing state in between.
    const el = this.cardEls.get(ed.objectId);
    if (el) {
      el.setAttribute('aria-hidden', 'true');
      el.removeAttribute('tabindex');
    }
    const frame = frameApi();
    if (opts.tellEngine) {
      try {
        frame?.setEmbedEditing?.(null);
      } catch (e) {
        console.warn('setEmbedEditing failed', e);
      }
    }
    const editor = this.editor;
    this.editor = null;
    if (ed.session && editor && !editor.isDestroyed) {
      // The static viewer comes back with what the editor showed, not with older content.
      const doc = editor.getJSON() as CachedDocJson['json']['doc'];
      this.qc.setQueryData<CachedDocJson>(docJsonKey(ed.item), (prev) => ({
        json: { schema_version: prev?.json.schema_version ?? 1, seq: prev?.json.seq ?? 0, doc },
        etag: prev?.etag ?? null,
      }));
    }
    this.publish();
    if (opts.focusFrame) {
      try {
        frame?.focus();
      } catch (e) {
        console.warn('frame.focus failed', e);
      }
    }
    const session = ed.session;
    const item = ed.item;
    this.closing = this.closing.then(async () => {
      if (session) {
        try {
          await session.flush();
        } catch (e) {
          console.warn('saving the embedded page failed', e);
        }
        session.destroy();
      }
      void this.qc.invalidateQueries({ queryKey: docJsonKey(item) });
    });
  }

  /** Resolves when the sessions of ended editing have flushed. */
  settled(): Promise<void> {
    return this.closing;
  }

  // ------------------------------------------------------------------------------------------- adding cards

  /**
   * §18.6: places a card for document `target` centred on `at` (the view centre or a frame point), titled
   * `title.trim() || 'Untitled'`; with `edit`, editing starts on it, otherwise the frame gets focus. `board`: the
   * board the action was started on (the "Text doc" button awaits a POST, during which the page may have moved to
   * another board): the card is placed only while that board is the open one. Null (and the error toast) when it
   * could not be placed.
   */
  insert(target: { id: string; title: string }, at: 'center' | FramePoint, opts: InsertOptions = {}): string | null {
    const frame = frameApi();
    const b = useEmbedStore.getState().board;
    let id: string | null = null;
    const onBoard = b !== null && (opts.board === undefined || opts.board === b.itemId);
    if (frame?.insertEmbed && b && onBoard && !b.readOnly && isItemId(target.id) && frame.current() === b.itemId) {
      try {
        id = frame.insertEmbed({ item: target.id, title: snapshotTitle(target.title), at });
      } catch (e) {
        console.warn('insertEmbed failed', e);
        id = null;
      }
    }
    if (id === null) {
      toast({ message: INSERT_FAILED, kind: 'error' });
      return null;
    }
    if (opts.edit) {
      this.start(id, target.id, null, null);
    } else {
      try {
        frame?.focus();
      } catch (e) {
        console.warn('frame.focus failed', e);
      }
    }
    return id;
  }

  /** A sidebar document dropped at `point` (frame px): a card there, titled from the cached detail if any. */
  dropItem(item: DraggedItem, point: FramePoint): void {
    const b = useEmbedStore.getState().board;
    if (!b || b.readOnly || frameApi()?.current() !== b.itemId) return;
    if (!isItemId(item.id)) return;
    const d = cachedItem(this.qc, item.id);
    if (d && d.item.kind !== 'doc') return;
    this.insert({ id: item.id, title: d ? d.item.title : item.title }, point);
  }

  /** A drop on the editing card (the overlay's capture listener), at a viewport point. */
  dropAtViewport(item: DraggedItem, clientX: number, clientY: number): void {
    const iframe = useHostStore.getState().iframe;
    const r = iframe ? iframe.getBoundingClientRect() : { left: 0, top: 0 };
    this.dropItem(item, viewportToFrame(r, { x: clientX, y: clientY }));
  }
}

// ------------------------------------------------------------------------------------------- for the board page

/** §18.6: see `EmbedController.insert`. Null without an overlay. */
export function insertCard(target: { id: string; title: string }, at: 'center' | FramePoint, opts: InsertOptions = {}): string | null {
  const c = instance;
  if (!c) {
    toast({ message: INSERT_FAILED, kind: 'error' });
    return null;
  }
  return c.insert(target, at, opts);
}

/** The board page's read-only state (BoardSurface). */
export function setEmbedReadOnly(itemId: string, readOnly: boolean): void {
  instance?.setReadOnly(itemId, readOnly);
}

/** The board page left the board (BoardSurface's cleanup): editing ends, the overlay serves no board. */
export function releaseEmbeds(itemId: string): void {
  instance?.release(itemId);
}
