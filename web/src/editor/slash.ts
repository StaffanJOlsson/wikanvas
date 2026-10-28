// The slash menu (docs/ARCHITECTURE.md §9.6.5): `/` opens it; typing filters; it closes after 3 characters that match
// nothing, or on Esc. Items: Text, Heading 1–3, Bulleted list, Numbered list, To-do list, Quote, Code block, Callout
// (Info, Tip, Warning, Danger), Divider, Table (3×3), Image, Mention a page, Embed board…, New board. An item whose
// callback the editor was not given is left out (a board card's editor has no board picker, §18.5 Editing step 2).
import { Extension, type Editor, type Range } from '@tiptap/react';
import { Suggestion, exitSuggestion } from '@tiptap/suggestion';
import { PluginKey } from '@tiptap/pm/state';
import { Popup } from './suggest';

export interface SlashContext {
  /** Opens the file picker and uploads the chosen images. */
  pickImage(editor: Editor): void;
  /** Opens the board picker; the chosen board is embedded. Without it, "Embed board…" is not offered. */
  pickBoard?(editor: Editor): void;
  /** Creates a child board of this document and embeds it. Without it, "New board" is not offered. */
  newBoard?(editor: Editor): void;
}

export interface SlashItem {
  key: string;
  title: string;
  /** More words it is found by. */
  keywords: string[];
  /** The context callback the item needs; the item is offered only when the context has it. */
  needs?: 'pickBoard' | 'newBoard';
  run(editor: Editor, range: Range, ctx: SlashContext): void;
}

const chain = (editor: Editor, range: Range) => editor.chain().focus().deleteRange(range);

export const SLASH_ITEMS: SlashItem[] = [
  { key: 'text', title: 'Text', keywords: ['paragraph', 'plain'], run: (e, r) => chain(e, r).setParagraph().run() },
  ...[1, 2, 3].map((level) => ({
    key: `h${level}`,
    title: `Heading ${level}`,
    keywords: [`h${level}`, 'title'],
    run: (e: Editor, r: Range) => chain(e, r).setHeading({ level: level as 1 | 2 | 3 }).run(),
  })),
  { key: 'bullet', title: 'Bulleted list', keywords: ['ul', 'unordered', 'bullet'], run: (e, r) => chain(e, r).toggleBulletList().run() },
  { key: 'ordered', title: 'Numbered list', keywords: ['ol', 'ordered', 'number'], run: (e, r) => chain(e, r).toggleOrderedList().run() },
  { key: 'todo', title: 'To-do list', keywords: ['task', 'checkbox', 'todo'], run: (e, r) => chain(e, r).toggleTaskList().run() },
  { key: 'quote', title: 'Quote', keywords: ['blockquote'], run: (e, r) => chain(e, r).toggleBlockquote().run() },
  { key: 'code', title: 'Code block', keywords: ['pre', 'snippet'], run: (e, r) => chain(e, r).toggleCodeBlock().run() },
  ...(['info', 'tip', 'warning', 'danger'] as const).map((variant) => ({
    key: `callout-${variant}`,
    title: `Callout: ${variant[0].toUpperCase()}${variant.slice(1)}`,
    keywords: ['callout', 'note', 'alert', variant],
    run: (e: Editor, r: Range) => chain(e, r).wrapIn('callout', { variant }).run(),
  })),
  { key: 'divider', title: 'Divider', keywords: ['hr', 'rule', 'line'], run: (e, r) => chain(e, r).setHorizontalRule().run() },
  {
    key: 'table',
    title: 'Table (3×3)',
    keywords: ['grid'],
    run: (e, r) => chain(e, r).insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run(),
  },
  { key: 'image', title: 'Image', keywords: ['picture', 'photo', 'upload'], run: (e, r, ctx) => { chain(e, r).run(); ctx.pickImage(e); } },
  { key: 'mention', title: 'Mention a page', keywords: ['link', 'page', '@'], run: (e, r) => chain(e, r).insertContent('@').run() },
  { key: 'embed', title: 'Embed board…', keywords: ['board', 'whiteboard', 'drawing'], needs: 'pickBoard', run: (e, r, ctx) => { chain(e, r).run(); ctx.pickBoard?.(e); } },
  { key: 'new-board', title: 'New board', keywords: ['board', 'whiteboard', 'create'], needs: 'newBoard', run: (e, r, ctx) => { chain(e, r).run(); ctx.newBoard?.(e); } },
];

/** The items a context offers: those whose needed callback it has. */
export function slashItemsFor(ctx: SlashContext, items: SlashItem[] = SLASH_ITEMS): SlashItem[] {
  return items.filter((i) => !i.needs || typeof ctx[i.needs] === 'function');
}

/** The items whose title or keywords contain the query (case-insensitive), title prefixes first. */
export function filterSlash(query: string, items: SlashItem[] = SLASH_ITEMS): SlashItem[] {
  const q = query.trim().toLowerCase();
  if (q === '') return items;
  const hits = items.filter((i) => i.title.toLowerCase().includes(q) || i.keywords.some((k) => k.includes(q)));
  return hits.sort((a, b) => Number(!a.title.toLowerCase().startsWith(q)) - Number(!b.title.toLowerCase().startsWith(q)));
}

/** The menu closes once 3 characters past the last query that matched something match nothing. */
export const CLOSE_AFTER = 3;

export class SlashFilter {
  private lastMatch = 0;

  /** The items for `query`, or null when the menu should close. */
  update(query: string, items: SlashItem[] = SLASH_ITEMS): SlashItem[] | null {
    const hits = filterSlash(query, items);
    if (hits.length > 0) {
      this.lastMatch = query.length;
      return hits;
    }
    return query.length - this.lastMatch >= CLOSE_AFTER ? null : [];
  }

  reset(): void {
    this.lastMatch = 0;
  }
}

export const slashKey = new PluginKey('slash');

/** The Suggestion plugin behind `/`, drawing into `popup`. */
export function SlashCommand(popup: Popup, ctx: SlashContext) {
  return Extension.create({
    name: 'slashCommand',
    addProseMirrorPlugins() {
      const editor = this.editor;
      const filter = new SlashFilter();
      const offered = slashItemsFor(ctx);
      let range: Range | null = null;
      const show = (query: string, r: Range, rect: DOMRect | null) => {
        range = r;
        const hits = filter.update(query, offered);
        if (hits === null) {
          popup.close();
          exitSuggestion(editor.view, slashKey);
          return;
        }
        popup.set({
          open: true,
          rect,
          empty: 'No matching blocks',
          items: hits.map((i) => ({
            key: i.key,
            label: i.title,
            run: () => {
              popup.close();
              if (range) i.run(editor, range, ctx);
            },
          })),
        });
      };
      return [
        Suggestion({
          editor,
          pluginKey: slashKey,
          char: '/',
          allowSpaces: false,
          startOfLine: false,
          items: () => [],
          render: () => ({
            onStart: (props) => {
              filter.reset();
              show(props.query, props.range, props.clientRect?.() ?? null);
            },
            onUpdate: (props) => show(props.query, props.range, props.clientRect?.() ?? null),
            onKeyDown: ({ event }) => {
              if (event.key === 'Escape') {
                popup.close();
                exitSuggestion(editor.view, slashKey);
                return true;
              }
              return popup.key(event.key);
            },
            onExit: () => popup.close(),
          }),
        }),
      ];
    },
  });
}
