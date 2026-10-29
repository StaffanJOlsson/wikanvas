// Following links (docs/ARCHITECTURE.md §9.6.5): an item link's DOM href is `/item/<uuid>` (the stored mark keeps
// `wikanvas://item/<uuid>`); a click follows a link when the document is read-only, Ctrl/Cmd+click while it is
// editable.
import { Editor } from '@tiptap/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { LinkOpener } from './extensions';
import { itemIdPath, linkTarget, type LinkTarget } from './links';
import { schemaExtensions } from './schema';

const ID = '0192f0a0-1111-7000-8000-00000000000a';
const ORIGIN = 'https://wiki.example';

let editor: Editor | null = null;
afterEach(() => {
  editor?.destroy();
  editor = null;
  document.body.innerHTML = '';
});

const doc = {
  type: 'doc',
  content: [
    {
      type: 'paragraph',
      content: [
        { type: 'text', text: 'item', marks: [{ type: 'link', attrs: { href: `wikanvas://item/${ID}`, itemId: ID } }] },
        { type: 'text', text: ' and ' },
        { type: 'text', text: 'web', marks: [{ type: 'link', attrs: { href: 'https://example.com/x', itemId: null } }] },
      ],
    },
  ],
};

function mount(open: (t: LinkTarget) => void, editable: boolean): Editor {
  const element = document.createElement('div');
  document.body.appendChild(element);
  editor = new Editor({ element, editable, extensions: [...schemaExtensions(), LinkOpener(open, ORIGIN)], content: doc });
  return editor;
}

/**
 * Clicks `el`; resolves whether the editor prevented the browser's own handling. The window then prevents it in any
 * case, since happy-dom would otherwise follow the link and fetch it.
 */
function click(el: Element, init: MouseEventInit = {}): { defaultPrevented: boolean } {
  let prevented = false;
  const last = (e: Event) => {
    prevented = e.defaultPrevented;
    e.preventDefault();
  };
  window.addEventListener('click', last);
  try {
    el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, button: 0, ...init }));
  } finally {
    window.removeEventListener('click', last);
  }
  return { defaultPrevented: prevented };
}

describe('item links', () => {
  it('render an openable href and keep wikanvas://item/<uuid> in the mark', () => {
    const e = mount(() => undefined, true);
    const a = e.view.dom.querySelector(`a[data-item-id="${ID}"]`);
    expect(a?.getAttribute('href')).toBe(`/item/${ID}`);
    expect(itemIdPath(ID)).toBe(`/item/${ID}`);
    const marks: unknown[] = [];
    e.state.doc.descendants((n) => {
      for (const m of n.marks) marks.push(m.attrs);
    });
    expect(marks[0]).toEqual({ href: `wikanvas://item/${ID}`, itemId: ID });
    // Copied as HTML and pasted back, the link is the item link again.
    const html = e.getHTML();
    expect(html).toContain(`href="/item/${ID}"`);
    e.commands.setContent(html);
    const again: unknown[] = [];
    e.state.doc.descendants((n) => {
      for (const m of n.marks) again.push(m.attrs);
    });
    expect(again[0]).toEqual({ href: `wikanvas://item/${ID}`, itemId: ID });
  });
});

describe('linkTarget', () => {
  it('routes item links and same-origin URLs, scrolls to fragments, opens the rest outside', () => {
    expect(linkTarget(`wikanvas://item/${ID}`, null, ORIGIN)).toEqual({ kind: 'route', to: `/item/${ID}` });
    expect(linkTarget('/doc/x-AbCdEfGh12', ID, ORIGIN)).toEqual({ kind: 'route', to: `/item/${ID}` });
    expect(linkTarget('/search?q=a', null, ORIGIN)).toEqual({ kind: 'route', to: '/search?q=a' });
    expect(linkTarget(`${ORIGIN}/doc/x-AbCdEfGh12#h-1`, null, ORIGIN)).toEqual({ kind: 'route', to: '/doc/x-AbCdEfGh12#h-1' });
    expect(linkTarget('#h-1', null, ORIGIN)).toEqual({ kind: 'hash', hash: '#h-1' });
    expect(linkTarget('https://example.com/', null, ORIGIN)).toEqual({ kind: 'external', url: 'https://example.com/' });
    expect(linkTarget('mailto:a@b.c', null, ORIGIN)).toEqual({ kind: 'external', url: 'mailto:a@b.c' });
    for (const bad of ['javascript:alert(1)', '//evil.example', '/\t/evil.example', 'data:text/html,x', '', null]) {
      expect(linkTarget(bad, null, ORIGIN), String(bad)).toBeNull();
    }
  });
});

describe('LinkOpener', () => {
  it('follows a link on a click when the document is read-only', () => {
    const open = vi.fn();
    const e = mount(open, false);
    const [item, web] = [...e.view.dom.querySelectorAll('a')];
    const ev = click(item);
    expect(ev.defaultPrevented).toBe(true);
    expect(open).toHaveBeenLastCalledWith({ kind: 'route', to: `/item/${ID}` });
    click(web);
    expect(open).toHaveBeenLastCalledWith({ kind: 'external', url: 'https://example.com/x' });
    // Ctrl/Cmd or Shift: the browser's own new tab or window, by the real href.
    const ctrl = click(item, { ctrlKey: true });
    expect(ctrl.defaultPrevented).toBe(false);
    expect(open).toHaveBeenCalledTimes(2);
  });

  it('places the caret on a plain click while editable, and follows on Ctrl/Cmd+click', () => {
    const open = vi.fn();
    const e = mount(open, true);
    const item = e.view.dom.querySelector('a') as HTMLAnchorElement;
    click(item);
    expect(open).not.toHaveBeenCalled();
    click(item, { ctrlKey: true });
    expect(open).toHaveBeenLastCalledWith({ kind: 'route', to: `/item/${ID}` });
    click(item, { metaKey: true });
    expect(open).toHaveBeenCalledTimes(2);
    // Not the primary button: nothing.
    click(item, { ctrlKey: true, button: 1 });
    expect(open).toHaveBeenCalledTimes(2);
  });
});
