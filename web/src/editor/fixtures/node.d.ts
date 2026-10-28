// The few Node.js APIs the fixture tests use (web/ has no @types/node; Vitest runs the tests in Node).
declare module 'node:fs' {
  export function readFileSync(path: URL | string): Uint8Array;
  export function readFileSync(path: URL | string, encoding: 'utf8'): string;
  export function writeFileSync(path: URL | string, data: string | Uint8Array): void;
  export function mkdirSync(path: URL | string, options?: { recursive?: boolean }): void;
  export function readdirSync(path: URL | string): string[];
  export function existsSync(path: URL | string): boolean;
}

declare const process: { env: Record<string, string | undefined> };
