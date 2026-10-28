// Schema v1 as data (docs/ARCHITECTURE.md §6.4): the live TipTap schema dumped in the shape of the API's `DocSchema`
// (`GET /api/v1/schema/doc`), which `fixtures/schema/v1.json` holds and `quire-core::schema` must equal. ProseMirror
// does not know attribute types, so they come from ATTR_TYPES; an attribute missing there fails the dump, so a new
// TipTap attribute cannot slip past the server's validator. Test-only code.
import type { Schema } from '@tiptap/pm/model';
import type { DocSchema } from '../../api/gen/DocSchema';
import { SCHEMA_VERSION } from '../schema';

/** The JSON type of every attribute, as `"<node or mark>.<attr>"` (or `"*.blockId"`). */
export const ATTR_TYPES: Record<string, string> = {
  '*.blockId': 'string | null',
  'heading.level': 'integer',
  'orderedList.start': 'integer',
  'orderedList.type': 'string | null',
  'codeBlock.language': 'string | null',
  'taskItem.checked': 'boolean',
  'tableHeader.colspan': 'integer',
  'tableHeader.rowspan': 'integer',
  'tableHeader.colwidth': 'integer[] | null',
  'tableHeader.align': '"left" | "center" | "right" | null',
  'tableCell.colspan': 'integer',
  'tableCell.rowspan': 'integer',
  'tableCell.colwidth': 'integer[] | null',
  'tableCell.align': '"left" | "center" | "right" | null',
  'image.src': 'string | null',
  'image.attachmentId': 'string | null',
  'image.width': 'number | null',
  'image.alt': 'string | null',
  'image.caption': 'string | null',
  'callout.variant': '"info" | "tip" | "warning" | "danger"',
  'mention.itemId': 'string | null',
  'boardEmbed.itemId': 'string | null',
  'link.href': 'string | null',
  'link.itemId': 'string | null',
};

function attrType(owner: string, name: string): string {
  const t = ATTR_TYPES[`${owner}.${name}`] ?? (name === 'blockId' ? ATTR_TYPES['*.blockId'] : undefined);
  if (t === undefined) throw new Error(`ATTR_TYPES has no type for ${owner}.${name}`);
  return t;
}

function attrs(owner: string, spec: Record<string, { default?: unknown }> | undefined) {
  return Object.entries(spec ?? {}).map(([name, a]) => ({
    name,
    type: attrType(owner, name),
    default: a.default === undefined ? null : a.default,
  }));
}

/** The schema in DocSchema shape: nodes in schema order, marks in rank order. */
export function dumpSchema(schema: Schema): DocSchema {
  const nodes = Object.values(schema.nodes).map((t) => ({
    name: t.name,
    group: (t.spec.group as string | undefined) ?? null,
    content: (t.spec.content as string | undefined) ?? null,
    marks: (t.spec.marks as string | undefined) ?? null,
    inline: t.isInline,
    atom: t.isAtom,
    attrs: attrs(t.name, t.spec.attrs as Record<string, { default?: unknown }> | undefined),
  }));
  const marks = Object.values(schema.marks)
    .sort((a, b) => a.rank - b.rank)
    .map((t) => ({
      name: t.name,
      rank: t.rank,
      excludes: (t.spec.excludes as string | undefined) ?? null,
      inclusive: (t.spec.inclusive as boolean | undefined) ?? null,
      attrs: attrs(t.name, t.spec.attrs as Record<string, { default?: unknown }> | undefined),
    }));
  return { schema_version: SCHEMA_VERSION, top_node: schema.topNodeType.name, nodes, marks } as DocSchema;
}
