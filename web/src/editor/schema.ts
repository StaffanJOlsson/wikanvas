// Wikanvas document schema v1 (docs/ARCHITECTURE.md §6.1, §6.4): the TipTap extensions that define the schema, shared
// by the editor, the headless fixture writer and the tests. Node and mark names are a contract with the server's
// walker (`wikanvas-core::ytext`, `wikanvas-core::schema`): renaming one needs schema_version 2 and a Y-level
// migration. The editor adds its node views, menus and input rules on top of these (editor/WikanvasEditor.tsx); nothing
// here touches the DOM beyond parseHTML/renderHTML.
import { isChangeOrigin } from '@tiptap/extension-collaboration';
import { CodeBlockLowlight } from '@tiptap/extension-code-block-lowlight';
import Highlight from '@tiptap/extension-highlight';
import Image from '@tiptap/extension-image';
import Link from '@tiptap/extension-link';
import Mention from '@tiptap/extension-mention';
import { Table, TableCell, TableHeader, TableRow } from '@tiptap/extension-table';
import TaskItem from '@tiptap/extension-task-item';
import TaskList from '@tiptap/extension-task-list';
import UniqueID from '@tiptap/extension-unique-id';
import { mergeAttributes, Node, wrappingInputRule, type AnyExtension, type NodeViewRenderer } from '@tiptap/react';
import StarterKit from '@tiptap/starter-kit';
import { isAllowedImageSrc, isAllowedLink, itemIdPath, LINK_SCHEMES, wikanvasItemId, wikanvasItemUrl } from './links';

export const SCHEMA_VERSION = 1;
/** The Y XmlFragment that holds the document. */
export const YFIELD = 'default';
/** The block id attribute UniqueID writes (heading anchors are `#h-<blockId>`). */
export const BLOCK_ID = 'blockId';

/** §6.1: every node name of schema v1 (plus ProseMirror's own `doc` and `text`). */
export const NODE_NAMES = [
  'paragraph',
  'heading',
  'bulletList',
  'orderedList',
  'listItem',
  'taskList',
  'taskItem',
  'blockquote',
  'codeBlock',
  'horizontalRule',
  'hardBreak',
  'table',
  'tableRow',
  'tableHeader',
  'tableCell',
  'image',
  'callout',
  'mention',
  'boardEmbed',
] as const;

/** §6.1: every mark name of schema v1. */
export const MARK_NAMES = ['bold', 'italic', 'underline', 'strike', 'code', 'highlight', 'link'] as const;

/** The block node types: each carries a `blockId` (UniqueID). Inline nodes (hardBreak, mention) do not. */
export const BLOCK_TYPES = [
  'paragraph',
  'heading',
  'bulletList',
  'orderedList',
  'listItem',
  'taskList',
  'taskItem',
  'blockquote',
  'codeBlock',
  'horizontalRule',
  'table',
  'tableRow',
  'tableHeader',
  'tableCell',
  'image',
  'callout',
  'boardEmbed',
] as const;

export const CALLOUT_VARIANTS = ['info', 'tip', 'warning', 'danger'] as const;
export type CalloutVariant = (typeof CALLOUT_VARIANTS)[number];

/** Image widths offered by the width handles (percent of the column, §9.6.5); null is the natural size. */
export const IMAGE_WIDTHS = [25, 50, 75, 100] as const;

/** A callout box: `callout{variant}` holding blocks (§6.1). */
export const Callout = Node.create({
  name: 'callout',
  group: 'block',
  content: 'block+',
  defining: true,
  addAttributes() {
    return {
      variant: {
        default: 'info',
        parseHTML: (el) => {
          const v = el.getAttribute('data-variant');
          return (CALLOUT_VARIANTS as readonly string[]).includes(v ?? '') ? v : 'info';
        },
        renderHTML: (attrs) => ({ 'data-variant': attrs.variant as string }),
      },
    };
  },
  parseHTML() {
    return [{ tag: 'div[data-callout]' }];
  },
  renderHTML({ HTMLAttributes }) {
    return ['div', mergeAttributes({ 'data-callout': '', class: 'callout' }, HTMLAttributes), 0];
  },
  addInputRules() {
    // `:::info ` (and tip, warning, danger) at the start of a block wraps it in a callout (§9.6.5).
    return [
      wrappingInputRule({
        find: /^:::(info|tip|warning|danger)\s$/,
        type: this.type,
        getAttributes: (m) => ({ variant: m[1] }),
      }),
    ];
  },
});

/** A board card: `boardEmbed{itemId}`, rendered as the page-0 SVG preview (never the engine, §9.6.5). */
export const BoardEmbed = Node.create({
  name: 'boardEmbed',
  group: 'block',
  atom: true,
  draggable: true,
  addAttributes() {
    return {
      itemId: {
        default: null,
        parseHTML: (el) => el.getAttribute('data-item-id'),
        renderHTML: (attrs) => (attrs.itemId ? { 'data-item-id': attrs.itemId as string } : {}),
      },
    };
  },
  parseHTML() {
    return [{ tag: 'div[data-board-embed]' }];
  },
  renderHTML({ HTMLAttributes }) {
    return ['div', mergeAttributes({ 'data-board-embed': '', class: 'board-embed' }, HTMLAttributes)];
  },
});

/**
 * Images: `image{src, attachmentId, width, alt, caption}` (§6.1), a block. An uploaded image has an `attachmentId`
 * and `src` `/api/v1/attachments/<id>`; an external one keeps `src` with `attachmentId: null` and renders as a
 * placeholder card that never loads it.
 */
export const WikanvasImage = Image.extend({
  addAttributes() {
    return {
      src: { default: null },
      attachmentId: {
        default: null,
        parseHTML: (el) => el.getAttribute('data-attachment-id'),
        renderHTML: (attrs) => (attrs.attachmentId ? { 'data-attachment-id': attrs.attachmentId as string } : {}),
      },
      width: {
        default: null,
        parseHTML: (el) => {
          const n = Number(el.getAttribute('data-width'));
          return (IMAGE_WIDTHS as readonly number[]).includes(n) ? n : null;
        },
        renderHTML: (attrs) => (attrs.width ? { 'data-width': String(attrs.width) } : {}),
      },
      alt: { default: null },
      caption: {
        default: null,
        parseHTML: (el) => el.getAttribute('title'),
        renderHTML: (attrs) => (attrs.caption ? { title: attrs.caption as string } : {}),
      },
    };
  },
  parseHTML() {
    // A pasted or dropped <img>: kept only within the §6.4 allow-list (never data: or javascript:). Its src stays,
    // with attachmentId null unless it is one of ours, and it renders as a placeholder (§6.1).
    return [
      {
        tag: 'img[src]',
        getAttrs: (el) => {
          const src = el.getAttribute('src');
          if (!isAllowedImageSrc(src)) return false;
          const m = /^\/api\/v1\/attachments\/([0-9A-Za-z-]+)$/.exec(src ?? '');
          return { src, attachmentId: el.getAttribute('data-attachment-id') ?? m?.[1] ?? null, alt: el.getAttribute('alt') };
        },
      },
    ];
  },
}).configure({ inline: false, allowBase64: false });

/** Mentions: `mention{itemId}` (§6.1); the label is the item's live title, never stored. */
export const WikanvasMention = Mention.extend({
  addAttributes() {
    return {
      itemId: {
        default: null,
        parseHTML: (el) => el.getAttribute('data-item-id'),
        renderHTML: (attrs) => (attrs.itemId ? { 'data-item-id': attrs.itemId as string } : {}),
      },
    };
  },
  renderText() {
    return '@';
  },
});

/** Links: `link{href, itemId}` (§6.1), limited to the §6.4 scheme allow-list. */
export const WikanvasLink = Link.extend({
  addAttributes() {
    return {
      href: {
        default: null,
        // An item link's DOM href is `/item/<uuid>` (below): read back, it is the item's `wikanvas://item/<uuid>`.
        parseHTML: (el) => {
          const id = el.getAttribute('data-item-id');
          return id && wikanvasItemId(wikanvasItemUrl(id)) ? wikanvasItemUrl(id.toLowerCase()) : el.getAttribute('href');
        },
        // The stored href stays as it is (`wikanvas://item/<uuid>` or a pasted Wikanvas URL); the DOM gets a URL a
        // browser can open, so a click, "open in new tab" and "copy link address" reach the item (§9.6.5).
        renderHTML: (attrs) => {
          const id = (attrs.itemId as string | null) ?? wikanvasItemId(attrs.href as string | null);
          return { href: id ? itemIdPath(id) : (attrs.href as string | null) };
        },
      },
      itemId: {
        default: null,
        parseHTML: (el) => el.getAttribute('data-item-id') ?? wikanvasItemId(el.getAttribute('href')),
        renderHTML: (attrs) => (attrs.itemId ? { 'data-item-id': attrs.itemId as string } : {}),
      },
    };
  },
}).configure({
  openOnClick: false,
  autolink: true,
  linkOnPaste: true,
  defaultProtocol: 'https',
  protocols: LINK_SCHEMES.filter((s) => s !== 'http' && s !== 'https' && s !== 'mailto').map((scheme) => ({
    scheme,
    optionalSlashes: false,
  })),
  isAllowedUri: (url) => isAllowedLink(url),
  shouldAutoLink: (url) => isAllowedLink(url),
  HTMLAttributes: { target: null, rel: 'noopener noreferrer nofollow', class: null },
});

/** Node views the editor draws its nodes with (the schema stays the same without them). */
export interface SchemaViews {
  image?: NodeViewRenderer;
  boardEmbed?: NodeViewRenderer;
  mention?: NodeViewRenderer;
  callout?: NodeViewRenderer;
  codeBlock?: NodeViewRenderer;
}

export interface SchemaOptions {
  views?: SchemaViews;
  /** Mention's suggestion (the `@` menu); without it, `@` types an @. */
  mentionSuggestion?: Record<string, unknown>;
  /** UniqueID's id generator (fixture mode: a seeded counter, §6.1). Default: UUID v4. */
  generateID?: () => string;
  /**
   * The lowlight instance code blocks highlight with (the editor's lazily loaded one, editor/lowlight.ts). Default:
   * one that highlights nothing (headless use).
   */
  lowlight?: unknown;
}

/** A lowlight stand-in that knows no language: code renders unhighlighted. */
export const NO_HIGHLIGHT = {
  highlight: () => ({ type: 'root', children: [] }),
  highlightAuto: () => ({ type: 'root', children: [] }),
  listLanguages: () => [] as string[],
  registered: () => false,
};

/** The extensions that make schema v1, in schema order. */
export function schemaExtensions(opts: SchemaOptions = {}): AnyExtension[] {
  const uniqueId = UniqueID.configure({
    types: [...BLOCK_TYPES],
    attributeName: BLOCK_ID,
    filterTransaction: (tr) => !isChangeOrigin(tr),
    ...(opts.generateID ? { generateID: opts.generateID } : {}),
  });
  const v = opts.views ?? {};
  const view = <T extends AnyExtension>(ext: T, r: NodeViewRenderer | undefined): AnyExtension =>
    r ? (ext as unknown as { extend: (c: object) => AnyExtension }).extend({ addNodeView: () => r }) : ext;
  return [
    StarterKit.configure({
      undoRedo: false,
      codeBlock: false,
      link: false,
      trailingNode: false,
      heading: { levels: [1, 2, 3, 4, 5, 6] },
    }),
    view(CodeBlockLowlight.configure({ lowlight: opts.lowlight ?? NO_HIGHLIGHT, defaultLanguage: null }), v.codeBlock),
    Highlight.configure({ multicolor: false }),
    WikanvasLink,
    TaskList,
    TaskItem.configure({ nested: true }),
    Table.configure({ resizable: false }),
    TableRow,
    TableHeader,
    TableCell,
    view(WikanvasImage, v.image),
    view(Callout, v.callout),
    view(opts.mentionSuggestion ? WikanvasMention.configure({ suggestion: opts.mentionSuggestion }) : WikanvasMention, v.mention),
    view(BoardEmbed, v.boardEmbed),
    uniqueId,
  ];
}

/** A seeded UniqueID generator: UUID-v4-shaped ids from a counter, so fixtures are byte-stable (§6.1). */
export function seededIds(seed = 1): () => string {
  let n = seed;
  return () => {
    const hex = (n++).toString(16).padStart(12, '0');
    return `00000000-0000-4000-8000-${hex}`;
  };
}
