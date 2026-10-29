// The live editor's extensions (docs/ARCHITECTURE.md §6.1, §9.6.5): schema v1 with its node views, Collaboration on
// the session's Y.Doc, the placeholder, heading anchors, the slash and mention menus, image paste/drop/upload with
// progress and retry, Wikanvas URLs pasted as item links, a board dropped from the sidebar as a board card, and
// Backspace at the start of an empty body back to the title. A board card's editor (`variant: 'embed'`, §18.5) has no
// heading anchors (two cards of one document would repeat the ids) and goes back to the card's own title input.
// `viewerExtensions` are the read-only static card's: the schema and the node views only.
import Collaboration from '@tiptap/extension-collaboration';
import FileHandler from '@tiptap/extension-file-handler';
import Placeholder from '@tiptap/extension-placeholder';
import { Extension, ReactNodeViewRenderer, type AnyExtension, type Editor } from '@tiptap/react';
import { Fragment, Slice } from '@tiptap/pm/model';
import { NodeSelection, Plugin, PluginKey } from '@tiptap/pm/state';
import { dropPoint } from '@tiptap/pm/transform';
import { Decoration, DecorationSet, type EditorView } from '@tiptap/pm/view';
import { exitSuggestion } from '@tiptap/suggestion';
import type * as Y from 'yjs';
import { api } from '../api/client';
import { searchTitles } from '../api/queries/docs';
import type { ItemDetail } from '../api/gen/ItemDetail';
import { DRAG_BOARD_TYPE, parseDraggedBoard } from '../board/embed/dragItem';
import { TITLE_INPUT_ID } from '../components/item/ItemHeader';
import { HeadingAnchors } from './contents';
import { isAllowedLink, linkTarget, sameInstanceUrlId, type LinkTarget } from './links';
import { lazyLowlight } from './lowlight';
import { schemaExtensions, YFIELD } from './schema';
import { SlashCommand, type SlashContext } from './slash';
import { Popup } from './suggest';
import { isImageFile, uploadFile } from './upload';
import { BoardEmbedView, CalloutView, CodeBlockView, ImageView, MentionView } from './views';

export interface EditorSetup {
  itemId: string;
  ydoc: Y.Doc;
  slash: Popup;
  mention: Popup;
  ctx: SlashContext;
  /** Follows a link (`LinkOpener`); without it, links are not followed from the editor. */
  openLink?(target: LinkTarget): void;
  /** `'embed'`: the editor of a board card (no heading anchors). Default `'page'`. */
  variant?: 'page' | 'embed';
  /** Where Backspace at the start of an empty body goes; default the page's `#item-title`. */
  titleInput?(): HTMLElement | null;
  /** Esc that nothing in the editor took (a board card's editor leaves editing, §18.5 Editing step 5). */
  onEscape?(): void;
}

// ------------------------------------------------------------------------------------------------- uploads

interface Uploading {
  id: string;
  progress: number;
  failed: boolean;
  retry?: () => void;
}

export const uploadKey = new PluginKey<DecorationSet>('wikanvasUploads');

function widget(u: Uploading): HTMLElement {
  const el = document.createElement('div');
  el.className = 'upload-placeholder';
  el.dataset.uploadId = u.id;
  el.contentEditable = 'false';
  if (u.failed) {
    el.textContent = 'Upload failed ';
    const b = document.createElement('button');
    b.type = 'button';
    b.textContent = 'Retry';
    b.onclick = () => u.retry?.();
    el.appendChild(b);
  } else {
    el.textContent = `Uploading… ${Math.round(u.progress * 100)}%`;
  }
  return el;
}

const UploadPlaceholders = Extension.create({
  name: 'wikanvasUploads',
  addProseMirrorPlugins() {
    return [
      new Plugin<DecorationSet>({
        key: uploadKey,
        state: {
          init: () => DecorationSet.empty,
          apply(tr, set) {
            let next = set.map(tr.mapping, tr.doc);
            const meta = tr.getMeta(uploadKey) as { add?: { pos: number; u: Uploading }; remove?: string } | undefined;
            if (meta?.remove) next = next.remove(next.find(undefined, undefined, (spec) => spec.id === meta.remove));
            if (meta?.add) {
              next = next.remove(next.find(undefined, undefined, (spec) => spec.id === meta.add?.u.id));
              next = next.add(tr.doc, [Decoration.widget(meta.add.pos, widget(meta.add.u), { id: meta.add.u.id })]);
            }
            return next;
          },
        },
        props: {
          decorations(state) {
            return this.getState(state);
          },
        },
      }),
    ];
  },
});

function placeholderPos(view: EditorView, id: string): number | null {
  const found = uploadKey.getState(view.state)?.find(undefined, undefined, (spec) => spec.id === id);
  return found && found.length > 0 ? found[0].from : null;
}

let uploadSeq = 0;

/** Uploads image files at `pos` with a progress placeholder; a failure offers Retry (§9.6.5). */
export function uploadImages(editor: Editor, itemId: string, files: File[], pos: number): void {
  for (const file of files.filter(isImageFile)) {
    const id = `upload-${++uploadSeq}`;
    const show = (u: Omit<Uploading, 'id'>, at?: number) => {
      const p = at ?? placeholderPos(editor.view, id);
      if (p === null) return;
      editor.view.dispatch(editor.state.tr.setMeta(uploadKey, { add: { pos: p, u: { id, ...u } } }));
    };
    const start = () => {
      show({ progress: 0, failed: false });
      uploadFile(file, file.name, { itemId, onProgress: (progress) => show({ progress, failed: false }) }).then(
        (att) => {
          const p = placeholderPos(editor.view, id);
          editor.view.dispatch(editor.state.tr.setMeta(uploadKey, { remove: id }));
          if (p === null || editor.isDestroyed) return;
          editor
            .chain()
            .insertContentAt(p, { type: 'image', attrs: { src: att.url, attachmentId: att.id, alt: file.name.replace(/\.[^.]+$/, '') } })
            .run();
        },
        () => show({ progress: 0, failed: true, retry: start }),
      );
    };
    show({ progress: 0, failed: false }, pos);
    start();
  }
}

// ------------------------------------------------------------------------------------------------- pasted URLs

/** A pasted same-instance item URL becomes `link{href, itemId}` once the item is found (§9.6.5). */
function WikanvasUrlPaste(origin: string) {
  return Extension.create({
    name: 'wikanvasUrlPaste',
    addProseMirrorPlugins() {
      const editor = this.editor;
      return [
        new Plugin({
          key: new PluginKey('wikanvasUrlPaste'),
          props: {
            handlePaste(view, event) {
              const text = event.clipboardData?.getData('text/plain')?.trim() ?? '';
              if (!text || /\s/.test(text)) return false;
              const urlId = sameInstanceUrlId(text, origin);
              if (!urlId || !isAllowedLink(text)) return false;
              const { from, to } = view.state.selection;
              void api<ItemDetail>('GET', `/items/${encodeURIComponent(urlId)}`).then(
                (d) => {
                  if (editor.isDestroyed) return;
                  const label = from === to ? d.item.title || text : null;
                  const chain = editor.chain().focus();
                  if (label) {
                    chain
                      .insertContentAt(from, { type: 'text', text: label, marks: [{ type: 'link', attrs: { href: text, itemId: d.item.id } }] })
                      .run();
                  } else {
                    chain.setTextSelection({ from, to }).setLink({ href: text }).updateAttributes('link', { itemId: d.item.id }).run();
                  }
                },
                () => {
                  if (!editor.isDestroyed) editor.chain().focus().insertContentAt(from, text).run();
                },
              );
              return true;
            },
          },
        }),
      ];
    },
  });
}

// ------------------------------------------------------------------------------------------------- dropped boards

/**
 * A sidebar board row dropped on the document becomes `boardEmbed{itemId}` (§9.6.5, D145), placed and selected the
 * way ProseMirror places a dropped node: at the nearest block boundary of the drop point (`dropPoint`). A drag with the
 * type whose data does not parse inserts nothing. A read-only editor never gets the drop (ProseMirror's drop handler
 * runs only while the view is editable).
 */
export const BoardDrop = Extension.create({
  name: 'wikanvasBoardDrop',
  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: new PluginKey('wikanvasBoardDrop'),
        props: {
          handleDrop(view, event) {
            const data = event.dataTransfer;
            if (!data || !Array.from(data.types).includes(DRAG_BOARD_TYPE)) return false;
            const board = parseDraggedBoard(data.getData(DRAG_BOARD_TYPE));
            const at = view.posAtCoords({ left: event.clientX, top: event.clientY });
            if (!board || !at) return true;
            const node = view.state.schema.nodes.boardEmbed.create({ itemId: board.id });
            const pos = dropPoint(view.state.doc, at.pos, new Slice(Fragment.from(node), 0, 0)) ?? at.pos;
            const tr = view.state.tr.replaceRangeWith(pos, pos, node);
            const $pos = tr.doc.resolve(pos);
            if ($pos.nodeAfter?.sameMarkup(node)) tr.setSelection(new NodeSelection($pos));
            view.focus();
            view.dispatch(tr.setMeta('uiEvent', 'drop'));
            return true;
          },
        },
      }),
    ];
  },
});

// ------------------------------------------------------------------------------------------------- following links

/**
 * Following a link mark (§9.6.5): a click when the document is read-only, Ctrl/Cmd+click while it is editable (a
 * plain click there places the caret, so link text stays editable). `open` gets the target (`linkTarget`). Read-only,
 * a click with Ctrl, Cmd or Shift is left to the browser (a new tab or window, by the anchor's real href). Mentions and
 * board cards are node views with their own links and are left alone.
 */
export function LinkOpener(open: (target: LinkTarget) => void, origin: string) {
  return Extension.create({
    name: 'wikanvasLinkOpener',
    addProseMirrorPlugins() {
      return [
        new Plugin({
          key: new PluginKey('wikanvasLinkOpener'),
          props: {
            handleDOMEvents: {
              click(view, event) {
                if (event.button !== 0) return false;
                const el = event.target instanceof Element ? event.target : (event.target as Node | null)?.parentElement;
                const a = el?.closest('a[href]');
                if (!a || !view.dom.contains(a) || a.closest('[data-node-view-wrapper]')) return false;
                const mod = event.metaKey || event.ctrlKey;
                if (view.editable ? !mod : mod || event.shiftKey) return false;
                event.preventDefault();
                const target = linkTarget(a.getAttribute('href'), a.getAttribute('data-item-id'), origin);
                if (target) open(target);
                return true;
              },
            },
          },
        }),
      ];
    },
  });
}

/** Opens a link target outside the router: a fragment scrolls to its anchor, an external URL opens in a new tab. */
export function openOutside(target: Exclude<LinkTarget, { kind: 'route' }>): void {
  if (target.kind === 'hash') {
    document.getElementById(decodeURIComponent(target.hash.slice(1)))?.scrollIntoView({ block: 'start' });
    return;
  }
  window.open(target.url, '_blank', 'noopener,noreferrer');
}

// ------------------------------------------------------------------------------------------------- the rest

function BackspaceToTitle(titleInput: () => HTMLElement | null) {
  return Extension.create({
    name: 'backspaceToTitle',
    addKeyboardShortcuts() {
      return {
        Backspace: ({ editor }) => {
          const { selection, doc } = editor.state;
          const empty = doc.childCount === 1 && doc.firstChild?.type.name === 'paragraph' && doc.firstChild.content.size === 0;
          if (!empty || !selection.empty || selection.from > 1) return false;
          const title = titleInput();
          if (!title) return false;
          title.focus();
          return true;
        },
      };
    },
  });
}

/**
 * Esc that nothing else in the editor took: ProseMirror itself prevents the default of every Esc (prosemirror-view's
 * `captureKeyDown`), so a listener around the editor cannot tell an Esc that a menu used from one that nobody did. The
 * lowest priority puts this keymap after every other plugin's keys, so the slash and mention menus close first.
 */
function EscapeOut(onEscape: () => void) {
  return Extension.create({
    name: 'wikanvasEscapeOut',
    priority: 1,
    addKeyboardShortcuts() {
      return {
        Escape: () => {
          onEscape();
          return true;
        },
      };
    },
  });
}

export const mentionKey = new PluginKey('wikanvasMention');

/** The `@` menu: a title search; the chosen item becomes a mention (§9.6.5). */
function mentionSuggestion(popup: Popup) {
  let last = 0;
  return {
    char: '@',
    pluginKey: mentionKey,
    allowSpaces: true,
    items: async ({ query }: { query: string }) => {
      const n = ++last;
      try {
        const hits = await searchTitles(query, 8);
        return n === last ? hits : [];
      } catch {
        return [];
      }
    },
    command: ({ editor, range, props }: { editor: Editor; range: { from: number; to: number }; props: { itemId: string } }) => {
      editor
        .chain()
        .focus()
        .insertContentAt(range, [{ type: 'mention', attrs: { itemId: props.itemId } }, { type: 'text', text: ' ' }])
        .run();
    },
    render: () => ({
      onStart: (p: SuggestRenderProps) => show(popup, p),
      onUpdate: (p: SuggestRenderProps) => show(popup, p),
      onKeyDown: ({ event, view }: { event: KeyboardEvent; view: EditorView }) => {
        if (event.key === 'Escape') {
          popup.close();
          exitSuggestion(view, mentionKey);
          return true;
        }
        return popup.key(event.key);
      },
      onExit: () => popup.close(),
    }),
  };
}

interface SuggestRenderProps {
  items: { item: { id: string; title: string; kind: string }; path: { title: string }[] }[];
  command(p: { itemId: string }): void;
  clientRect?: (() => DOMRect | null) | null;
}

function show(popup: Popup, p: SuggestRenderProps) {
  popup.set({
    open: true,
    rect: p.clientRect?.() ?? null,
    empty: 'No pages found',
    items: p.items.map((h) => ({
      key: h.item.id,
      label: h.item.title || 'Untitled',
      hint: h.path.map((c) => c.title).join(' / '),
      run: () => p.command({ itemId: h.item.id }),
    })),
  });
}

/** The node views of the live editor and of the static viewers. */
function nodeViews() {
  return {
    image: ReactNodeViewRenderer(ImageView),
    boardEmbed: ReactNodeViewRenderer(BoardEmbedView),
    mention: ReactNodeViewRenderer(MentionView, { as: 'span' }),
    callout: ReactNodeViewRenderer(CalloutView),
    codeBlock: ReactNodeViewRenderer(CodeBlockView),
  };
}

/** The extensions of a read-only viewer (a board's static card, §18.5): the schema and its node views. */
export function viewerExtensions(): AnyExtension[] {
  return schemaExtensions({ lowlight: lazyLowlight, views: nodeViews() });
}

/** Every extension of the live editor. */
export function editorExtensions(s: EditorSetup): AnyExtension[] {
  const embed = s.variant === 'embed';
  return [
    ...schemaExtensions({
      lowlight: lazyLowlight,
      mentionSuggestion: mentionSuggestion(s.mention),
      views: nodeViews(),
    }),
    Collaboration.configure({ document: s.ydoc, field: YFIELD }),
    Placeholder.configure({ placeholder: 'Type / for commands', showOnlyWhenEditable: true }),
    ...(embed ? [] : [HeadingAnchors]),
    UploadPlaceholders,
    FileHandler.configure({
      onPaste: (editor, files) => uploadImages(editor, s.itemId, files, editor.state.selection.from),
      onDrop: (editor, files, pos) => uploadImages(editor, s.itemId, files, pos),
    }),
    WikanvasUrlPaste(typeof window !== 'undefined' ? window.location.origin : ''),
    BoardDrop,
    ...(s.openLink ? [LinkOpener(s.openLink, typeof window !== 'undefined' ? window.location.origin : '')] : []),
    SlashCommand(s.slash, s.ctx),
    BackspaceToTitle(s.titleInput ?? (() => document.getElementById(TITLE_INPUT_ID))),
    ...(s.onEscape ? [EscapeOut(s.onEscape)] : []),
  ];
}
