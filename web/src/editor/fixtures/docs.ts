// The fixture documents of fixtures/ydoc/* (PLAN W4 "Fixtures"): together they use every node, mark and attribute of
// schema v1, so the server's golden tests (ytext, convert::pm) cover all of it. `getting-started` is the "Welcome to
// Wikanvas" guide of docs/ARCHITECTURE.md §9.6.7 that the seed stores; `writing-in-wikanvas` is its child page.
import type { JSONContent } from '@tiptap/react';

/** Placeholder item ids in the seed documents; the seed replaces them with the real ids (`docs::init_seed`). */
export const SEED_WRITING_ID = '00000000-0000-7000-8000-000000000001';
export const SEED_BOARD_ID = '00000000-0000-7000-8000-000000000002';

/** Item and attachment ids the other fixtures reference. */
export const ITEM_A = '0192f0a0-1111-7000-8000-00000000000a';
export const ITEM_B = '0192f0a0-2222-7000-8000-00000000000b';
export const BOARD_C = '0192f0a0-3333-7000-8000-00000000000c';
export const ATTACHMENT_1 = '0192f0a0-4444-7000-8000-000000000001';

type Mark = { type: string; attrs?: Record<string, unknown> };

const t = (text: string, ...marks: (string | Mark)[]): JSONContent =>
  marks.length === 0
    ? { type: 'text', text }
    : { type: 'text', text, marks: marks.map((m) => (typeof m === 'string' ? { type: m } : m)) };
const p = (...content: (JSONContent | string)[]): JSONContent =>
  content.length === 0
    ? { type: 'paragraph' }
    : { type: 'paragraph', content: content.map((c) => (typeof c === 'string' ? t(c) : c)) };
const h = (level: number, text: string): JSONContent => ({ type: 'heading', attrs: { level }, content: [t(text)] });
const li = (...content: JSONContent[]): JSONContent => ({ type: 'listItem', content });
const ul = (...items: JSONContent[]): JSONContent => ({ type: 'bulletList', content: items });
const ol = (start: number, ...items: JSONContent[]): JSONContent => ({ type: 'orderedList', attrs: { start }, content: items });
const task = (checked: boolean, ...content: JSONContent[]): JSONContent => ({ type: 'taskItem', attrs: { checked }, content });
const tasks = (...items: JSONContent[]): JSONContent => ({ type: 'taskList', content: items });
const callout = (variant: string, ...content: JSONContent[]): JSONContent => ({ type: 'callout', attrs: { variant }, content });
const code = (language: string | null, text: string): JSONContent => ({
  type: 'codeBlock',
  attrs: { language },
  content: text === '' ? undefined : [t(text)],
});
const link = (href: string, itemId: string | null = null): Mark => ({ type: 'link', attrs: { href, itemId } });
const mention = (itemId: string): JSONContent => ({ type: 'mention', attrs: { itemId } });
const br: JSONContent = { type: 'hardBreak' };
const cell = (type: 'tableCell' | 'tableHeader', attrs: Record<string, unknown>, ...content: JSONContent[]) => ({
  type,
  attrs,
  content,
});
const row = (...cells: JSONContent[]): JSONContent => ({ type: 'tableRow', content: cells });

export const FIXTURE_DOCS: Record<string, JSONContent[]> = {
  // What the editor leaves in a new document: one empty paragraph.
  empty: [p()],

  marks: [
    p('Plain, ', t('bold', 'bold'), ', ', t('italic', 'italic'), ', ', t('underline', 'underline'), ', ',
      t('strike', 'strike'), ', ', t('code', 'code'), ' and ', t('highlight', 'highlight'), '.'),
    p(t('bold italic', 'bold', 'italic'), t(' bold only', 'bold'), ' ', t('all', 'bold', 'italic', 'underline', 'strike', 'highlight')),
    p('A ', t('web link', link('https://example.com/a?b=c#d')), ', a ', t('mail', link('mailto:someone@example.com')),
      ', an ', t('item link', link(`wikanvas://item/${ITEM_A}`, ITEM_A)), ' and a ', t('bold link', 'bold', link('/doc/x-AbCdEfGh12')), '.'),
    p(t(' spaced ', 'italic'), 'edges', t(' ', 'bold')),
    p('Escapes: *stars* _under_ `ticks` [brackets] <angle> # hash 1. one'),
  ],

  blocks: [
    h(1, 'Heading 1'),
    h(2, 'Heading 2'),
    h(3, 'Heading 3'),
    h(4, 'Heading 4'),
    h(5, 'Heading 5'),
    h(6, 'Heading 6'),
    { type: 'blockquote', content: [p('Quoted line one.'), p('Quoted ', t('line', 'bold'), ' two.')] },
    code('rust', 'fn main() {\n    println!("```");\n}'),
    code(null, 'plain code'),
    { type: 'horizontalRule' },
    callout('info', p('An info callout.')),
    callout('tip', p('A tip.'), ul(li(p('with a list')))),
    callout('warning', p('A warning.')),
    callout('danger', p('Danger!')),
    p('Line one', br, 'line two', br, 'line three'),
    p(),
    p('After an empty paragraph.'),
  ],

  lists: [
    ul(li(p('One')), li(p('Two'), ul(li(p('Two A')), li(p('Two B')))), li(p('Three'))),
    ol(1, li(p('First')), li(p('Second'))),
    ol(3, li(p('Third')), li(p('Fourth'), p('A second paragraph.'))),
    tasks(task(false, p('Open task')), task(true, p('Done task'), tasks(task(false, p('Nested task'))))),
    ul(li(p('Adjacent bullet list'))),
    ul(li(p('Another bullet list'))),
  ],

  table: [
    {
      type: 'table',
      content: [
        row(cell('tableHeader', {}, p('Name')), cell('tableHeader', {}, p('Kind')), cell('tableHeader', {}, p('Notes'))),
        row(cell('tableCell', {}, p('Wikanvas')), cell('tableCell', {}, p('Wiki')), cell('tableCell', { colwidth: [180] }, p('Self-hosted'))),
        row(cell('tableCell', { colspan: 2 }, p('Spans two columns')), cell('tableCell', {}, p('One'), p('Two'))),
        row(cell('tableCell', {}, p()), cell('tableCell', {}, p(t('bold', 'bold'))), cell('tableCell', { align: 'right' }, p('right'))),
      ],
    },
    p('After the table.'),
  ],

  media: [
    p('An uploaded image:'),
    {
      type: 'image',
      attrs: {
        src: `/api/v1/attachments/${ATTACHMENT_1}`,
        attachmentId: ATTACHMENT_1,
        width: 50,
        alt: 'A diagram',
        caption: 'The caption',
      },
    },
    { type: 'image', attrs: { src: 'https://example.com/remote.png', alt: 'Remote' } },
    p('See ', mention(ITEM_B), ' and ', mention(ITEM_A), '.'),
    { type: 'boardEmbed', attrs: { itemId: BOARD_C } },
  ],

  unicode: [
    p('a😀é — é ', t('ñ', 'bold'), ' 中文 עברית 👩‍👩‍👧'),
    h(2, 'Ünïcödé'),
    code('text', 'tab\there\nline'),
  ],

  'getting-started': [
    p('Wikanvas keeps your documents and whiteboards in one place. This page is a short tour; delete it whenever you like.'),
    h(2, 'Collections and pages'),
    p('Everything lives in ', t('collections', 'bold'), ', listed in the sidebar on the left. A collection holds '
      + 'documents and boards, and every page can hold more pages inside it, nested as deep as you need. Use the '
      + '+ next to a collection or a page to add one, and drag pages in the sidebar to reorder or nest them.'),
    p('Pages are saved as you type. There is nothing to publish: what you write is there for you right away.'),
    h(2, 'Finding things'),
    ul(
      li(p('Press ', t('Ctrl+K', 'code'), ' (', t('⌘K', 'code'), ' on a Mac) to jump to any page by its title.')),
      li(p('Search looks through the text of every document and board.')),
      li(p('Star a page to keep it at the top of the sidebar.')),
    ),
    h(2, 'Archive and Trash'),
    p('Archive a page you no longer need day to day: it leaves the sidebar and the search results (tick Include '
      + 'archived in Search to find it), and you can bring it back from the Archive. Deleted pages go to the Trash, '
      + 'where they stay for 30 days before they are removed for good.'),
    h(2, 'Backups'),
    p('Wikanvas backs up its database on a schedule into the ', t('backups', 'code'), ' folder of its data directory ('
      + '', t('/data/backups', 'code'), ' in the Docker image). Settings → Data exports everything as one file.'),
    h(2, 'Next'),
    p('Read ', mention(SEED_WRITING_ID), ' for the editor, or open the example board below.'),
    { type: 'boardEmbed', attrs: { itemId: SEED_BOARD_ID } },
  ],

  'writing-in-wikanvas': [
    p('The editor turns a few keystrokes into structure.'),
    h(2, 'The slash menu'),
    p('Type ', t('/', 'code'), ' at the start of a line to insert headings, lists, to-dos, quotes, code, callouts, '
      + 'tables, images, mentions and boards. Keep typing to filter the list.'),
    h(2, 'Markdown shortcuts'),
    ul(
      li(p(t('# ', 'code'), ' to ', t('### ', 'code'), ' start a heading.')),
      li(p(t('- ', 'code'), ' or ', t('1. ', 'code'), ' start a list, ', t('[] ', 'code'), ' a to-do.')),
      li(p(t('> ', 'code'), ' starts a quote and ', t('```', 'code'), ' a code block.')),
      li(p(t('**bold**', 'code'), ', ', t('*italic*', 'code'), ', ', t('`code`', 'code'), ', ', t('~~strike~~', 'code'),
        ' and ', t('==highlight==', 'code'), ' format text as you type.')),
    ),
    tasks(task(true, p('Try a to-do')), task(false, p('Tick it off'))),
    h(2, 'Mentions and links'),
    p('Type ', t('@', 'code'), ' and a few letters to mention another page. Mentions always show the page\'s current '
      + 'title. Paste a link to a Wikanvas page and it stays a link to that page, even after a rename.'),
    h(2, 'Images'),
    p('Paste or drop an image, or choose Image in the slash menu. Drag the handles to resize it and add a caption.'),
    h(2, 'Callouts'),
    callout('info', p('Start a line with ', t(':::info', 'code'), ' and a space for a box like this one.')),
    callout('tip', p('There are tip, warning and danger boxes too.')),
    h(2, 'Tables'),
    {
      type: 'table',
      content: [
        row(cell('tableHeader', {}, p('Key')), cell('tableHeader', {}, p('In a table'))),
        row(cell('tableCell', {}, p(t('Tab', 'code'))), cell('tableCell', {}, p('Moves to the next cell'))),
        row(cell('tableCell', {}, p(t('Shift+Tab', 'code'))), cell('tableCell', {}, p('Moves back'))),
      ],
    },
    p('Use the cell menu to add or remove rows and columns.'),
  ],
};
