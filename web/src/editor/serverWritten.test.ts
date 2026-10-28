// Every server-written document (fixtures/ydoc/server/*.bin, made by `convert::tests::write_server_fixtures`) opens
// in a headless TipTap editor, and `editor.getJSON()` equals its `.pm.json` with no schema repair: the editor writes
// nothing back (docs/ARCHITECTURE.md §6.4).
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { openDoc } from './fixtures/writer';

const DIR = '../fixtures/ydoc/server/';

describe('server-written documents', () => {
  const names = existsSync(DIR) ? readdirSync(DIR).filter((f) => f.endsWith('.bin')).map((f) => f.slice(0, -4)) : [];

  it('exist', () => {
    expect(names.length).toBeGreaterThan(5);
  });

  for (const name of names) {
    it(`${name} opens unchanged`, () => {
      const bin = readFileSync(`${DIR}${name}.bin`);
      const want = JSON.parse(readFileSync(`${DIR}${name}.pm.json`, 'utf8'));
      const { pm, changed } = openDoc(bin);
      expect(pm).toEqual(want);
      expect(changed).toBe(false);
    });
  }
});
