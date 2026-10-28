// How BoardFrameHost gets a BoardFrameApi (docs/ARCHITECTURE.md §9.5, §10.4): it creates the iframe of the engine's
// frame page and, after `load`, calls `iframe.contentWindow.quireBoardFrame.connect(events)` (same origin, plain JS
// calls). Tests swap the driver for the fake frame (`fakeFrame.ts`) with `setFrameDriver`.
import type { BoardFrameApi, FrameErrorKind, FrameEvents } from './bridge';

/** Why a connection failed. `missing`: the page loaded without `window.quireBoardFrame` (e.g. a 404 page). */
export interface ConnectFailure {
  kind: FrameErrorKind;
  message: string;
  missing?: boolean;
}

export interface FrameDriver {
  /** A new iframe for the frame page at `src`, not yet in the document. */
  create(src: string): HTMLIFrameElement;
  /** Connects to the frame page once it has loaded. Rejects with a `ConnectFailure`. Called before the iframe is
   *  added to the document, so no `load` is missed. */
  connect(iframe: HTMLIFrameElement, events: FrameEvents): Promise<BoardFrameApi>;
}

/** A rejection value of `connect` (the frame's `{kind, message}`, an Error, anything) as a ConnectFailure. */
export function toConnectFailure(e: unknown): ConnectFailure {
  if (e && typeof e === 'object' && 'kind' in e && typeof (e as { kind: unknown }).kind === 'string') {
    const { kind, message, missing } = e as Partial<ConnectFailure>;
    return { kind: kind as FrameErrorKind, message: message ?? String(kind), missing };
  }
  return { kind: 'start', message: e instanceof Error ? e.message : String(e) };
}

export const realDriver: FrameDriver = {
  create(src) {
    const iframe = document.createElement('iframe');
    iframe.className = 'board-frame';
    iframe.title = 'Whiteboard';
    iframe.setAttribute('allow', 'fullscreen; clipboard-read; clipboard-write');
    iframe.src = src;
    return iframe;
  },
  connect(iframe, events) {
    return new Promise<BoardFrameApi>((resolve, reject) => {
      iframe.addEventListener(
        'load',
        () => {
          const qbf = iframe.contentWindow?.quireBoardFrame;
          if (!qbf) {
            reject({ kind: 'download', message: 'The whiteboard engine could not be loaded.', missing: true });
            return;
          }
          qbf.connect(events).then(resolve, (e: unknown) => reject(toConnectFailure(e)));
        },
        { once: true },
      );
    });
  },
};

let driver: FrameDriver = realDriver;

/** The driver BoardFrameHost uses. */
export function frameDriver(): FrameDriver {
  return driver;
}

/** Replaces the driver (tests: the fake frame); `null` restores the real one. */
export function setFrameDriver(d: FrameDriver | null): void {
  driver = d ?? realDriver;
}
