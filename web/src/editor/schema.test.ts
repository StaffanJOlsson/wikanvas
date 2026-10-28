// Schema v1's node and mark names are the §6.1 list (docs/ARCHITECTURE.md §6.1): the server's walker depends on them.
import { Editor } from '@tiptap/react';
import { describe, expect, it } from 'vitest';
import { BLOCK_TYPES, MARK_NAMES, NODE_NAMES, schemaExtensions } from './schema';

const SECTION_6_1_NODES = [
  'paragraph', 'heading', 'bulletList', 'orderedList', 'listItem', 'taskList', 'taskItem', 'blockquote', 'codeBlock',
  'horizontalRule', 'hardBreak', 'table', 'tableRow', 'tableHeader', 'tableCell', 'image', 'callout', 'mention',
  'boardEmbed',
];
const SECTION_6_1_MARKS = ['bold', 'italic', 'underline', 'strike', 'code', 'highlight', 'link'];

describe('schema v1', () => {
  const editor = new Editor({ extensions: schemaExtensions() });
  const schema = editor.schema;

  it('has exactly the §6.1 nodes and marks', () => {
    const nodes = Object.keys(schema.nodes).filter((n) => n !== 'doc' && n !== 'text');
    expect(nodes.sort()).toEqual([...SECTION_6_1_NODES].sort());
    expect(Object.keys(schema.marks).sort()).toEqual([...SECTION_6_1_MARKS].sort());
    expect([...NODE_NAMES].sort()).toEqual([...SECTION_6_1_NODES].sort());
    expect([...MARK_NAMES].sort()).toEqual([...SECTION_6_1_MARKS].sort());
  });

  it('gives every block node a blockId and no inline node one', () => {
    for (const [name, type] of Object.entries(schema.nodes)) {
      if (name === 'doc' || name === 'text') continue;
      const has = 'blockId' in (type.spec.attrs ?? {});
      expect(has, name).toBe(!type.isInline);
      expect((BLOCK_TYPES as readonly string[]).includes(name), name).toBe(!type.isInline);
    }
  });

  it('has the §6.1 attributes', () => {
    const attrs = (n: string) => Object.keys(schema.nodes[n].spec.attrs ?? {}).sort();
    expect(attrs('image')).toEqual(['alt', 'attachmentId', 'blockId', 'caption', 'src', 'width']);
    expect(attrs('callout')).toEqual(['blockId', 'variant']);
    expect(attrs('mention')).toEqual(['itemId']);
    expect(attrs('boardEmbed')).toEqual(['blockId', 'itemId']);
    expect(Object.keys(schema.marks.link.spec.attrs ?? {}).sort()).toEqual(['href', 'itemId']);
    expect(schema.marks.code.spec.excludes).toBe('_');
  });
});
