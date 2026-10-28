// When a board opens read-only in BoardPreview instead of the editor (docs/ARCHITECTURE.md §9.5): no engine in
// `/api/v1/config`; a browser the frame cannot run in (no WebGL2, WebAssembly, BigInt or crypto.getRandomValues: the
// frame reports `unsupported` or `webgl`); an engine that failed to download or start; a panic; and, by default, a
// phone (a coarse pointer under 768 px), which gets an "Open editor" button.
import type { Config } from '../api/gen/Config';
import type { Failure } from './host';

export type FallbackKind = 'no-engine' | 'unsupported' | 'failed' | 'panic' | 'phone';

export interface FallbackReason {
  kind: FallbackKind;
  message: string;
}

export interface FallbackInput {
  /** The server config; undefined while it loads. */
  config: Pick<Config, 'inkboard'> | null | undefined;
  /** The frame host's failure, if any. */
  failure: Failure | null;
  /** No BoardFrameHost is mounted. */
  hostless: boolean;
  /** A phone, and the user has not asked for the editor. */
  phone: boolean;
}

/** Why the board is shown read-only, or null when the editor should open. */
export function fallbackReason({ config, failure, hostless, phone }: FallbackInput): FallbackReason | null {
  if (config === null || (config !== undefined && config.inkboard === null) || hostless) {
    return { kind: 'no-engine', message: 'Whiteboard engine not installed' };
  }
  if (failure) {
    switch (failure.kind) {
      case 'panic':
        return { kind: 'panic', message: 'The whiteboard engine stopped.' };
      case 'unsupported':
      case 'webgl':
        return {
          kind: 'unsupported',
          message: `This browser cannot run the whiteboard editor: ${failure.message}`,
        };
      default:
        return { kind: 'failed', message: `The whiteboard editor could not start: ${failure.message}` };
    }
  }
  if (phone) return { kind: 'phone', message: 'Boards open read-only on small touch screens.' };
  return null;
}
