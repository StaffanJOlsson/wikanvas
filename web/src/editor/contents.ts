// The Contents column (docs/ARCHITECTURE.md §6.1, §9.6.4): computed from the document's `heading` nodes (no
// TableOfContents extension). Shown with at least two headings; anchors are `#h-<blockId>`; the active heading is
// the last one whose top is above 30% of the viewport height. `HeadingAnchors` gives each heading element its id
// with a node decoration, so the schema does not change.
import { Extension, type JSONContent } from '@tiptap/react';
import type { Node as PMNode } from '@tiptap/pm/model';
import { Plugin, PluginKey } from '@tiptap/pm/state';
import { Decoration, DecorationSet } from '@tiptap/pm/view';
import { BLOCK_ID } from './schema';

export interface Heading {
  blockId: string;
  level: number;
  text: string;
}

/** The scroll-spy line: a heading is passed once its top is above this share of the viewport height. */
export const SPY_THRESHOLD = 0.3;
/** The Contents column appears with this many headings. */
export const MIN_HEADINGS = 2;

/** `h-<blockId>`: a heading's element id (its link is `#h-<blockId>`). */
export function headingAnchor(blockId: string): string {
  return `h-${blockId}`;
}

function textOf(node: JSONContent): string {
  if (node.type === 'text') return node.text ?? '';
  if (node.type === 'hardBreak') return ' ';
  return (node.content ?? []).map(textOf).join('');
}

/** The headings of a document (top level only: headings inside quotes, callouts or lists are not listed). */
export function headingsOf(doc: JSONContent): Heading[] {
  return (doc.content ?? [])
    .filter((n) => n.type === 'heading' && typeof n.attrs?.[BLOCK_ID] === 'string')
    .map((n) => ({
      blockId: n.attrs?.[BLOCK_ID] as string,
      level: Number(n.attrs?.level ?? 1),
      text: textOf(n).trim() || 'Untitled heading',
    }));
}

/** The same from a ProseMirror document. */
export function headingsOfNode(doc: PMNode): Heading[] {
  const out: Heading[] = [];
  doc.forEach((n) => {
    if (n.type.name === 'heading' && typeof n.attrs[BLOCK_ID] === 'string') {
      out.push({ blockId: n.attrs[BLOCK_ID] as string, level: Number(n.attrs.level), text: n.textContent.trim() || 'Untitled heading' });
    }
  });
  return out;
}

/** Whether the Contents column shows. */
export function showContents(headings: readonly Heading[]): boolean {
  return headings.length >= MIN_HEADINGS;
}

/**
 * The active heading: the last one whose top (relative to the viewport) is above 30% of its height, else the first.
 * Once the page is scrolled to its end (`atEnd`), the last heading in view is active instead: the headings of the last
 * screen can never reach the 30% line (D147). `tops` are in document order.
 */
export function activeHeading(
  tops: readonly { blockId: string; top: number }[],
  viewportHeight: number,
  atEnd = false,
): string | null {
  if (tops.length === 0) return null;
  if (atEnd) {
    const inView = tops.filter((t) => t.top < viewportHeight);
    if (inView.length > 0) return inView[inView.length - 1].blockId;
  }
  const line = viewportHeight * SPY_THRESHOLD;
  let active = tops[0].blockId;
  for (const t of tops) {
    if (t.top <= line) active = t.blockId;
    else break;
  }
  return active;
}

/**
 * Whether the scroller that holds `el` (its nearest ancestor that scrolls vertically and overflows, else the document)
 * is scrolled to its end. Browsers round scroll positions (Chrome stopped at 392 of 393 on a live page), so the last
 * 2 px count as the end. A scroller with nothing to scroll is never at its end.
 */
export function scrolledToEnd(el: Element | null): boolean {
  const atEnd = (s: Element) => s.scrollTop + s.clientHeight >= s.scrollHeight - 2;
  for (let p = el?.parentElement ?? null; p; p = p.parentElement) {
    const overflow = getComputedStyle(p).overflowY;
    if ((overflow === 'auto' || overflow === 'scroll') && p.scrollHeight > p.clientHeight) return atEnd(p);
  }
  const root = document.scrollingElement;
  return !!root && root.scrollHeight > root.clientHeight && atEnd(root);
}

/** Element ids `h-<blockId>` on heading elements, as node decorations. */
export const HeadingAnchors = Extension.create({
  name: 'headingAnchors',
  addProseMirrorPlugins() {
    const build = (doc: PMNode) => {
      const decos: Decoration[] = [];
      doc.forEach((n, pos) => {
        const id = n.attrs?.[BLOCK_ID];
        if (n.type.name === 'heading' && typeof id === 'string') {
          decos.push(Decoration.node(pos, pos + n.nodeSize, { id: headingAnchor(id) }));
        }
      });
      return DecorationSet.create(doc, decos);
    };
    return [
      new Plugin({
        key: new PluginKey('headingAnchors'),
        state: {
          init: (_, state) => build(state.doc),
          apply: (tr, old) => (tr.docChanged ? build(tr.doc) : old),
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
