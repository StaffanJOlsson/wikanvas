// The editor's menus (docs/ARCHITECTURE.md §9.6.5): the suggestion popups (slash, mention), the bubble menu on a text
// selection, the table cell menu, and the block handle.
import DragHandle from '@tiptap/extension-drag-handle-react';
import type { Node as PMNode } from '@tiptap/pm/model';
import { type Editor, useEditorState } from '@tiptap/react';
import { BubbleMenu } from '@tiptap/react/menus';
import { Bold, Code, GripVertical, Highlighter, Italic, Link as LinkIcon, Strikethrough, Underline } from 'lucide-react';
import { useState, type ReactNode } from 'react';
import { headingAnchor } from './contents';
import { isAllowedLink } from './links';
import { BLOCK_ID } from './schema';
import { usePopup, type Popup } from './suggest';

/** A slash or mention popup, under the caret. */
export function SuggestPopup({ popup, label }: { popup: Popup; label: string }) {
  const s = usePopup(popup);
  if (!s.open) return null;
  const style = s.rect ? { left: s.rect.left, top: s.rect.bottom + 4 } : undefined;
  return (
    <div className="suggest-popup" role="listbox" aria-label={label} style={style}>
      {s.items.length === 0 && <div className="suggest-empty">{s.empty ?? 'Nothing found'}</div>}
      {s.items.map((item, i) => (
        <button
          key={item.key}
          type="button"
          role="option"
          aria-selected={i === s.selected}
          className="suggest-item"
          onMouseDown={(e) => {
            e.preventDefault();
            item.run();
          }}
        >
          <span>{item.label}</span>
          {item.hint && <span className="suggest-hint">{item.hint}</span>}
        </button>
      ))}
    </div>
  );
}

/** Asks for a URL and links the selection with it, within the §6.4 allow-list. */
function promptLink(editor: Editor) {
  const prev = (editor.getAttributes('link').href as string | undefined) ?? '';
  const href = window.prompt('Link URL', prev);
  if (href === null) return;
  if (href.trim() === '') {
    editor.chain().focus().extendMarkRange('link').unsetLink().run();
    return;
  }
  if (!isAllowedLink(href.trim())) {
    window.alert('Links can use http, https, mailto or a path in this wiki.');
    return;
  }
  editor.chain().focus().extendMarkRange('link').setLink({ href: href.trim() }).run();
}

const TURN_INTO: { label: string; run(e: Editor): void }[] = [
  { label: 'Text', run: (e) => e.chain().focus().setParagraph().run() },
  { label: 'Heading 1', run: (e) => e.chain().focus().setHeading({ level: 1 }).run() },
  { label: 'Heading 2', run: (e) => e.chain().focus().setHeading({ level: 2 }).run() },
  { label: 'Heading 3', run: (e) => e.chain().focus().setHeading({ level: 3 }).run() },
  { label: 'Bulleted list', run: (e) => e.chain().focus().toggleBulletList().run() },
  { label: 'Numbered list', run: (e) => e.chain().focus().toggleOrderedList().run() },
  { label: 'To-do list', run: (e) => e.chain().focus().toggleTaskList().run() },
  { label: 'Quote', run: (e) => e.chain().focus().toggleBlockquote().run() },
];

/** Bold, italic, underline, strike, code, highlight, link and "Turn into" on a text selection. */
export function SelectionMenu({ editor }: { editor: Editor }) {
  const marks = useEditorState({
    editor,
    selector: ({ editor: e }) => ({
      bold: e.isActive('bold'),
      italic: e.isActive('italic'),
      underline: e.isActive('underline'),
      strike: e.isActive('strike'),
      code: e.isActive('code'),
      highlight: e.isActive('highlight'),
      link: e.isActive('link'),
    }),
  });
  const btn = (name: keyof typeof marks, label: string, icon: ReactNode, run: () => void) => (
    <button type="button" className="icon-button" aria-label={label} aria-pressed={marks[name]} onMouseDown={(e) => e.preventDefault()} onClick={run}>
      {icon}
    </button>
  );
  return (
    <BubbleMenu
      editor={editor}
      className="bubble-menu"
      shouldShow={({ editor: e, state }) => e.isEditable && !state.selection.empty && !e.isActive('codeBlock') && !e.isActive('image')}
    >
      {btn('bold', 'Bold', <Bold size={14} />, () => editor.chain().focus().toggleBold().run())}
      {btn('italic', 'Italic', <Italic size={14} />, () => editor.chain().focus().toggleItalic().run())}
      {btn('underline', 'Underline', <Underline size={14} />, () => editor.chain().focus().toggleUnderline().run())}
      {btn('strike', 'Strikethrough', <Strikethrough size={14} />, () => editor.chain().focus().toggleStrike().run())}
      {btn('code', 'Code', <Code size={14} />, () => editor.chain().focus().toggleCode().run())}
      {btn('highlight', 'Highlight', <Highlighter size={14} />, () => editor.chain().focus().toggleHighlight().run())}
      {btn('link', 'Link', <LinkIcon size={14} />, () => promptLink(editor))}
      <select
        aria-label="Turn into"
        value=""
        onChange={(e) => TURN_INTO.find((t) => t.label === e.target.value)?.run(editor)}
      >
        <option value="" disabled>
          Turn into
        </option>
        {TURN_INTO.map((t) => (
          <option key={t.label}>{t.label}</option>
        ))}
      </select>
    </BubbleMenu>
  );
}

/** Rows and columns of the table the caret is in. */
export function TableMenu({ editor }: { editor: Editor }) {
  const items: [string, () => boolean][] = [
    ['Row above', () => editor.chain().focus().addRowBefore().run()],
    ['Row below', () => editor.chain().focus().addRowAfter().run()],
    ['Delete row', () => editor.chain().focus().deleteRow().run()],
    ['Column left', () => editor.chain().focus().addColumnBefore().run()],
    ['Column right', () => editor.chain().focus().addColumnAfter().run()],
    ['Delete column', () => editor.chain().focus().deleteColumn().run()],
    ['Header row', () => editor.chain().focus().toggleHeaderRow().run()],
    ['Delete table', () => editor.chain().focus().deleteTable().run()],
  ];
  return (
    <BubbleMenu
      editor={editor}
      pluginKey="tableMenu"
      className="table-menu"
      shouldShow={({ editor: e }) => e.isEditable && e.isActive('table')}
    >
      {items.map(([label, run]) => (
        <button key={label} type="button" onMouseDown={(e) => e.preventDefault()} onClick={() => run()}>
          {label}
        </button>
      ))}
    </BubbleMenu>
  );
}

/** The block handle: drag to reorder; a menu with Turn into, Duplicate, Copy link (headings) and Delete. */
export function BlockHandle({ editor }: { editor: Editor }) {
  const [current, setCurrent] = useState<{ node: PMNode; pos: number } | null>(null);
  const [open, setOpen] = useState(false);
  const act = (f: (node: PMNode, pos: number) => void) => () => {
    setOpen(false);
    if (current) f(current.node, current.pos);
  };
  return (
    <DragHandle
      editor={editor}
      className="block-handle"
      onNodeChange={({ node, pos }) => {
        setCurrent(node ? { node, pos } : null);
        setOpen(false);
      }}
    >
      <button type="button" className="icon-button" aria-label="Block menu" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <GripVertical size={14} aria-hidden />
      </button>
      {open && current && (
        <div className="block-menu" role="menu">
          {TURN_INTO.map((t) => (
            <button
              key={t.label}
              type="button"
              role="menuitem"
              onClick={act((_n, pos) => {
                editor.chain().setTextSelection(pos + 1).run();
                t.run(editor);
              })}
            >
              Turn into {t.label}
            </button>
          ))}
          <button
            type="button"
            role="menuitem"
            onClick={act((node, pos) => {
              const json = node.toJSON() as { attrs?: Record<string, unknown> };
              editor.chain().focus().insertContentAt(pos + node.nodeSize, { ...json, attrs: { ...json.attrs, [BLOCK_ID]: null } }).run();
            })}
          >
            Duplicate
          </button>
          {current.node.type.name === 'heading' && typeof current.node.attrs[BLOCK_ID] === 'string' && (
            <button
              type="button"
              role="menuitem"
              onClick={act((node) => {
                const url = `${window.location.origin}${window.location.pathname}#${headingAnchor(node.attrs[BLOCK_ID] as string)}`;
                void navigator.clipboard?.writeText(url);
              })}
            >
              Copy link
            </button>
          )}
          <button
            type="button"
            role="menuitem"
            onClick={act((node, pos) => editor.chain().focus().deleteRange({ from: pos, to: pos + node.nodeSize }).run())}
          >
            Delete
          </button>
        </div>
      )}
    </DragHandle>
  );
}
