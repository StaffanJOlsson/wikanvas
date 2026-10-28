// The headless fixture writer (docs/ARCHITECTURE.md §6.1, §6.4; PLAN W4 "Fixtures"): a TipTap editor bound to a Y.Doc
// with a fixed clientID and seeded UniqueID ids, so the output is byte-stable. Each fixture document is set with
// `setContent` (a normal local edit that the binding writes into Y), then read back in every shape the server's
// golden tests compare: the Yjs state (`.bin`), y-tiptap's raw JSON (`.y.json`), `editor.getJSON()` (`.pm.json`),
// and the expected output of the server's Y walk (`.txt`, `.links.json`, computed here independently of the Rust
// walker). Test-only code: nothing in the app imports it.
import Collaboration from '@tiptap/extension-collaboration';
import { Editor, type JSONContent } from '@tiptap/react';
import { yXmlFragmentToProsemirrorJSON } from '@tiptap/y-tiptap';
import * as Y from 'yjs';
import { schemaExtensions, seededIds, YFIELD } from '../schema';

/** The Y clientID of every fixture document. */
export const FIXTURE_CLIENT_ID = 1;

export interface WrittenDoc {
  bin: Uint8Array;
  /** y-tiptap's `yXmlFragmentToProsemirrorJSON` (raw: stored attrs only, marks in Yjs order). */
  raw: unknown;
  /** `editor.getJSON()` (normalized). */
  pm: JSONContent;
  /** The plain text the server derives (§6.1). */
  text: string;
  links: WalkLinks;
}

export interface WalkLinks {
  /** Item links, sorted by target then kind, without duplicates. */
  links: { target_id: string; kind: 'mention' | 'link' | 'embed' }[];
  /** Image attachment ids, sorted, without duplicates. */
  attachment_ids: string[];
  /** Only empty paragraphs (§6.1). */
  is_empty: boolean;
}

/** A mounted editor on `doc` (happy-dom), configured as the app's editor is for schema and ids. */
export function headlessEditor(doc: Y.Doc, generateID?: () => string): Editor {
  return new Editor({
    extensions: [...schemaExtensions({ generateID }), Collaboration.configure({ document: doc, field: YFIELD })],
  });
}

/** Writes `content` (the `doc` node's children) into a fresh fixture Y.Doc through a headless editor. */
export function writeDoc(content: JSONContent[]): WrittenDoc {
  const ydoc = new Y.Doc();
  ydoc.clientID = FIXTURE_CLIENT_ID;
  const editor = headlessEditor(ydoc, seededIds());
  try {
    editor.commands.setContent({ type: 'doc', content });
    const pm = editor.getJSON();
    const raw = yXmlFragmentToProsemirrorJSON(ydoc.getXmlFragment(YFIELD));
    return { bin: Y.encodeStateAsUpdate(ydoc), raw, pm, ...walk(pm) };
  } finally {
    editor.destroy();
  }
}

/** Opens Yjs state in a headless editor (no id generation is expected) and returns `editor.getJSON()`. */
export function openDoc(bin: Uint8Array): { pm: JSONContent; changed: boolean } {
  const ydoc = new Y.Doc();
  Y.applyUpdate(ydoc, bin, 'server');
  const before = Y.encodeStateVector(ydoc);
  const editor = headlessEditor(ydoc, () => {
    throw new Error('a server-written document needs no new block id');
  });
  try {
    const pm = editor.getJSON();
    const after = Y.encodeStateVector(ydoc);
    const changed = before.length !== after.length || before.some((b, i) => b !== after[i]);
    return { pm, changed };
  } finally {
    editor.destroy();
  }
}

const BLOCK_TEXT = new Set(['paragraph', 'heading', 'codeBlock']);

/**
 * The §6.1 walk over normalized JSON, as the server defines it: every textblock (paragraph, heading, code block) is
 * one line, a hard break ends a line, an image's caption is a line of its own; table rows are lines whose cells are
 * separated by `\t` (a cell's blocks joined by a space); empty lines are dropped and the rest joined by `\n`.
 */
export function walk(doc: JSONContent): Omit<WrittenDoc, 'bin' | 'raw' | 'pm'> {
  const lines: string[] = [];
  const links = new Map<string, WalkLinks['links'][number]>();
  const attachments = new Set<string>();
  const addLink = (target: unknown, kind: 'mention' | 'link' | 'embed') => {
    if (typeof target === 'string' && target !== '') links.set(`${target}\u0000${kind}`, { target_id: target, kind });
  };
  const inline = (nodes: JSONContent[] | undefined): string[] => {
    const parts = [''];
    for (const n of nodes ?? []) {
      if (n.type === 'text') {
        parts[parts.length - 1] += n.text ?? '';
        for (const m of n.marks ?? []) if (m.type === 'link') addLink(m.attrs?.itemId, 'link');
      } else if (n.type === 'hardBreak') {
        parts.push('');
      } else if (n.type === 'mention') {
        addLink(n.attrs?.itemId, 'mention');
      }
    }
    return parts;
  };
  const cellText = (cell: JSONContent): string => {
    const out: string[] = [];
    const visit = (n: JSONContent) => {
      if (BLOCK_TEXT.has(n.type ?? '')) out.push(...inline(n.content).filter((s) => s !== ''));
      else if (n.type === 'image') image(n, (s) => out.push(s));
      else if (n.type === 'boardEmbed') addLink(n.attrs?.itemId, 'embed');
      else for (const c of n.content ?? []) visit(c);
    };
    for (const c of cell.content ?? []) visit(c);
    return out.join(' ');
  };
  const image = (n: JSONContent, push: (s: string) => void) => {
    if (typeof n.attrs?.attachmentId === 'string') attachments.add(n.attrs.attachmentId);
    if (typeof n.attrs?.caption === 'string' && n.attrs.caption !== '') push(n.attrs.caption);
  };
  const block = (n: JSONContent) => {
    const type = n.type ?? '';
    if (BLOCK_TEXT.has(type)) lines.push(...inline(n.content));
    else if (type === 'image') image(n, (s) => lines.push(s));
    else if (type === 'boardEmbed') addLink(n.attrs?.itemId, 'embed');
    else if (type === 'tableRow') lines.push((n.content ?? []).map(cellText).join('\t'));
    else for (const c of n.content ?? []) block(c);
  };
  for (const c of doc.content ?? []) block(c);
  const isEmpty = (doc.content ?? []).every((c) => c.type === 'paragraph' && (c.content ?? []).length === 0);
  return {
    text: lines.filter((l) => l.replace(/\t/g, '') !== '').join('\n'),
    links: {
      links: [...links.values()].sort((a, b) =>
        a.target_id === b.target_id ? a.kind.localeCompare(b.kind) : a.target_id < b.target_id ? -1 : 1,
      ),
      attachment_ids: [...attachments].sort(),
      is_empty: isEmpty,
    },
  };
}
