// The fixture writer's run (`pnpm --dir web fixtures` sets WRITE_FIXTURES=1, PLAN W4 acceptance): writes
// fixtures/ydoc/<name>.{bin,y.json,pm.json,txt,links.json} and fixtures/schema/v1.json. Without WRITE_FIXTURES it
// checks that the committed files are exactly what the writer produces, so `pnpm test` catches drift.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { Editor } from '@tiptap/react';
import { describe, expect, it } from 'vitest';
import { MARK_NAMES, NODE_NAMES, schemaExtensions } from '../schema';
import { FIXTURE_DOCS } from './docs';
import { dumpSchema } from './schemaDump';
import { writeDoc } from './writer';

const WRITE = process.env.WRITE_FIXTURES === '1';
// Relative to web/, where Vitest runs (`pnpm --dir web …`); under happy-dom `import.meta.url` is not a file URL.
const FIXTURES = '../fixtures/';
const YDOC = `${FIXTURES}ydoc/`;
const SCHEMA = `${FIXTURES}schema/`;

const json = (v: unknown) => `${JSON.stringify(v, null, 2)}\n`;
const sameBytes = (a: Uint8Array, b: Uint8Array) => a.length === b.length && a.every((x, i) => x === b[i]);

function output(path: string, data: string | Uint8Array): void {
  if (WRITE) {
    writeFileSync(path, data);
    return;
  }
  expect(existsSync(path), `${path} is missing: run scripts/dev.sh pnpm --dir web fixtures`).toBe(true);
  const committed = readFileSync(path);
  const want = typeof data === 'string' ? new TextEncoder().encode(data) : data;
  expect(sameBytes(committed, want), `${path} is out of date: run scripts/dev.sh pnpm --dir web fixtures`).toBe(
    true,
  );
}

describe('fixtures', () => {
  if (WRITE) {
    mkdirSync(YDOC, { recursive: true });
    mkdirSync(SCHEMA, { recursive: true });
  }

  it('schema/v1.json is the live schema', () => {
    const editor = new Editor({ extensions: schemaExtensions() });
    try {
      const schema = dumpSchema(editor.schema);
      const names = schema.nodes.map((n) => n.name).filter((n) => n !== 'doc' && n !== 'text');
      expect([...names].sort()).toEqual([...NODE_NAMES].sort());
      expect(schema.marks.map((m) => m.name).sort()).toEqual([...MARK_NAMES].sort());
      output(`${SCHEMA}v1.json`, json(schema));
    } finally {
      editor.destroy();
    }
  });

  for (const [name, content] of Object.entries(FIXTURE_DOCS)) {
    it(`ydoc/${name}`, () => {
      const doc = writeDoc(content);
      // Byte-stable: a second run gives the same bytes.
      expect(sameBytes(writeDoc(content).bin, doc.bin)).toBe(true);
      output(`${YDOC}${name}.bin`, doc.bin);
      output(`${YDOC}${name}.y.json`, json(doc.raw));
      output(`${YDOC}${name}.pm.json`, json(doc.pm));
      output(`${YDOC}${name}.txt`, doc.text);
      output(`${YDOC}${name}.links.json`, json(doc.links));
    });
  }
});
