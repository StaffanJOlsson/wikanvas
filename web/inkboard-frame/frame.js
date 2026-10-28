// @ts-check
// Quire's board frame page (docs/ARCHITECTURE.md §10.4), served as /inkboard/{build}/frame.html + frame.js next to the
// engine (inkboard.js, inkboard_bg.wasm, icons.bin) and manifest.json, all written by scripts/inkboard.sh build. It
// implements `QuireBoardFrame` of src/board/bridge.ts on top of the Inkboard JS API (§10.3, inkboard.d.ts): the SPA's
// BoardFrameHost (same origin) calls `iframe.contentWindow.quireBoardFrame.connect(events)` once the frame has loaded.
//
// - connect: feature checks, manifest.json, the wasm download counted for onProgress, then init() (whose wasm `main`
//   installs globalThis.inkboardEmbed). It resolves to the BoardFrameApi, or rejects {kind, message} after onError.
// - open: the first call mounts on #board, later calls swap the board (setBoard + setReadOnly + setName). Calls run one
//   after the other, and each waits until the page is visible (a hidden page draws no frame). snapshot() and
//   isDirty() never wait.
// - Events carry the itemId of the board in the engine; the host ignores events of another item.
// - Mod+K and Mod+. go to the host (onHostKey); a second Esc within 500 ms focuses the parent and sends Escape. F5 and
//   Mod+R keep the browser's reload.
// - A WebAssembly.RuntimeError reaching window `error` or `unhandledrejection` reports onError{kind:'panic'} (once,
//   shared with the engine's own panic report).
// - pagehide (not into the back/forward cache) releases the WebGL context.

/** @import { BoardFrameApi, ConnectError, FrameErrorKind, FrameEvents, OpenArgs, QuireBoardFrame } from '../src/board/bridge.ts' */
/** @import { InkboardEmbed, InkboardHandle } from './inkboard.js' */
import init from './inkboard.js';

/**
 * The parts of the dist's manifest.json (patches/inkboard/README.md) the frame reads.
 * @typedef {{ format_version: number, patchset: string, files: Record<string, { raw: number }> }} Manifest
 */

const WASM = 'inkboard_bg.wasm';
const DOUBLE_ESCAPE_MS = 500;

const canvas = /** @type {HTMLCanvasElement} */ (document.getElementById('board'));

/** @type {FrameEvents} */
let events = {};
/** @type {Promise<BoardFrameApi> | null} */
let connecting = null;
/** @type {InkboardHandle | null} */
let handle = null;
/** The item whose board is in the engine. @type {string | null} */
let current = null;
/** open() calls, one after the other. @type {Promise<void>} */
let queue = Promise.resolve();
let active = true;
let panicked = false;

/**
 * Reports a failure through onError (a panic only once) and returns what to throw or reject with.
 * @param {FrameErrorKind} kind
 * @param {string} message
 * @param {string | null} itemId
 * @returns {ConnectError}
 */
function fail(kind, message, itemId) {
  if (kind === 'panic') {
    if (panicked) return { kind, message };
    panicked = true;
  }
  try {
    events.onError?.({ itemId, kind, message });
  } catch (error) {
    console.error(error);
  }
  return { kind, message };
}

/** @param {unknown} error */
function messageOf(error) {
  if (error instanceof Error) return error.message;
  if (error && typeof error === 'object' && 'message' in error) return String(error.message);
  return String(error);
}

/**
 * The engine's `{kind, message}` rejections (`webgl`, `load`, `start`), anything else as `fallback`.
 * @param {unknown} error
 * @param {FrameErrorKind} fallback
 * @returns {ConnectError}
 */
function engineError(error, fallback) {
  if (error && typeof error === 'object' && 'kind' in error) {
    const kind = error.kind;
    if (kind === 'webgl' || kind === 'load' || kind === 'start') return { kind, message: messageOf(error) };
  }
  return { kind: fallback, message: messageOf(error) };
}

/** What the engine cannot run without (checked before the download). */
function missingFeatures() {
  const missing = [];
  if (typeof WebAssembly !== 'object' || typeof WebAssembly.instantiateStreaming !== 'function') missing.push('WebAssembly');
  if (typeof BigInt !== 'function') missing.push('BigInt');
  if (typeof globalThis.crypto?.getRandomValues !== 'function') missing.push('crypto.getRandomValues');
  if (typeof ReadableStream !== 'function') missing.push('ReadableStream');
  return missing;
}

function hasWebGL2() {
  try {
    return document.createElement('canvas').getContext('webgl2') !== null;
  } catch {
    return false;
  }
}

/** Resolves once the page is visible (at once when it is). */
function whenVisible() {
  if (document.visibilityState === 'visible') return Promise.resolve();
  return new Promise((resolve) => {
    const check = () => {
      if (document.visibilityState !== 'visible') return;
      document.removeEventListener('visibilitychange', check);
      resolve(undefined);
    };
    document.addEventListener('visibilitychange', check);
  });
}

/** @returns {Promise<Manifest>} */
async function loadManifest() {
  let response;
  try {
    response = await fetch('manifest.json');
  } catch (error) {
    throw fail('download', `manifest.json: ${messageOf(error)}`, null);
  }
  if (!response.ok) throw fail('download', `manifest.json: HTTP ${response.status}`, null);
  /** @type {Manifest} */
  let manifest;
  try {
    manifest = await response.json();
  } catch (error) {
    throw fail('download', `manifest.json: ${messageOf(error)}`, null);
  }
  if (typeof manifest?.files?.[WASM]?.raw !== 'number' || typeof manifest.patchset !== 'string') {
    throw fail('start', `manifest.json does not describe ${WASM}`, null);
  }
  return manifest;
}

/**
 * Downloads and starts the engine: onProgress counts the decoded bytes against the manifest's raw size, so the last
 * call has `loaded == total` whatever the transfer encoding.
 * @param {number} total
 */
async function startEngine(total) {
  let response;
  try {
    response = await fetch(WASM);
  } catch (error) {
    throw fail('download', `${WASM}: ${messageOf(error)}`, null);
  }
  if (!response.ok || !response.body) throw fail('download', `${WASM}: HTTP ${response.status}`, null);
  const reader = response.body.getReader();
  /** @type {unknown} */
  let readError = null;
  let loaded = 0;
  const progress = () => {
    try {
      events.onProgress?.({ loaded, total });
    } catch (error) {
      console.error(error);
    }
  };
  progress();
  const stream = new ReadableStream({
    async pull(controller) {
      try {
        const { done, value } = await reader.read();
        if (done) {
          controller.close();
          return;
        }
        loaded += value.byteLength;
        progress();
        controller.enqueue(value);
      } catch (error) {
        readError = error;
        controller.error(error);
      }
    },
    cancel(reason) {
      return reader.cancel(reason);
    },
  });
  try {
    await init({ module_or_path: new Response(stream, { headers: { 'Content-Type': 'application/wasm' } }) });
  } catch (error) {
    if (readError !== null) throw fail('download', `${WASM}: ${messageOf(readError)}`, null);
    throw fail('start', messageOf(error), null);
  }
  const engine = globalThis.inkboardEmbed;
  if (!engine) throw fail('start', 'the engine did not install inkboardEmbed', null);
  return engine;
}

/**
 * Mounts the first board. Resolves with the handle once the engine has drawn its first frame (onReady).
 * @param {InkboardEmbed} engine
 * @param {OpenArgs} a
 * @returns {Promise<InkboardHandle>}
 */
function mountBoard(engine, a) {
  return new Promise((resolve, reject) => {
    /** @type {InkboardHandle | null} */
    let mounted = null;
    let ready = false;
    engine
      .mount(canvas, {
        board: a.board,
        name: a.name,
        theme: a.theme,
        readOnly: a.readOnly,
        onReady() {
          ready = true;
          if (mounted) resolve(mounted);
        },
        onChange(e) {
          if (current !== null) events.onChange?.({ itemId: current, revision: e.revision });
        },
        onSave() {
          if (current !== null) events.onSave?.(current);
        },
        onError(e) {
          fail('panic', e.message, current);
        },
      })
      .then((h) => {
        mounted = h;
        if (ready) resolve(h);
      }, reject);
  });
}

/**
 * @param {InkboardEmbed} engine
 * @param {OpenArgs} a
 */
async function openNow(engine, a) {
  if (panicked) throw { kind: 'panic', message: 'the whiteboard engine stopped' };
  await whenVisible();
  if (handle === null) {
    current = a.itemId;
    try {
      handle = await mountBoard(engine, a);
    } catch (error) {
      current = null;
      const { kind, message } = engineError(error, 'start');
      throw fail(kind, message, a.itemId);
    }
    if (!active) handle.setActive(false);
  } else {
    try {
      handle.setBoard(a.board, a.name);
    } catch (error) {
      // the previous board stays in the engine, and so does its itemId
      const { kind, message } = engineError(error, 'load');
      throw fail(kind, message, a.itemId);
    }
    current = a.itemId;
    handle.setReadOnly(a.readOnly);
    handle.setName(a.name);
    handle.setTheme(a.theme);
  }
  events.onReady?.(a.itemId);
}

/** @returns {Promise<BoardFrameApi>} */
async function start() {
  const missing = missingFeatures();
  if (missing.length > 0) throw fail('unsupported', `this browser lacks ${missing.join(', ')}`, null);
  if (!hasWebGL2()) throw fail('webgl', 'WebGL 2 is not available', null);
  const manifest = await loadManifest();
  const engine = await startEngine(manifest.files[WASM].raw);
  if (engine.formatVersion !== manifest.format_version) {
    throw fail('start', `the engine writes format ${engine.formatVersion}, the manifest says ${manifest.format_version}`, null);
  }
  /** @type {BoardFrameApi} */
  const api = {
    engineVersion: `${engine.version}.quire.${manifest.patchset}`,
    formatVersion: engine.formatVersion,
    open(a) {
      const run = queue.then(() => openNow(engine, a));
      queue = run.catch(() => undefined);
      return run;
    },
    current: () => current,
    snapshot() {
      if (handle === null || current === null) throw new Error('no board is open');
      return { itemId: current, ...handle.snapshot() };
    },
    isDirty: () => handle?.isDirty() ?? false,
    markSaved: (revision) => handle?.markSaved(revision),
    setTheme: (theme) => handle?.setTheme(theme),
    setReadOnly: (readOnly) => handle?.setReadOnly(readOnly),
    setName: (name) => handle?.setName(name),
    setActive(on) {
      active = on;
      handle?.setActive(on);
    },
    focus() {
      if (handle) handle.focus();
      else canvas.focus();
    },
    fitView: () => handle?.fitView(),
  };
  return Object.freeze(api);
}

/** @type {QuireBoardFrame} */
const quireBoardFrame = {
  connect(ev) {
    events = ev ?? {};
    if (connecting === null) {
      const attempt = start();
      connecting = attempt;
      // a failed download may be retried by connecting again; anything else stays failed
      attempt.catch((/** @type {unknown} */ error) => {
        if (connecting === attempt && isDownload(error)) connecting = null;
      });
    }
    return connecting;
  },
};

/** @param {unknown} error */
function isDownload(error) {
  return !!error && typeof error === 'object' && 'kind' in error && error.kind === 'download';
}

/* ---------------------------------------------------------------------------------------------- page listeners */

const isMac = /Mac|iPhone|iPad/.test(navigator.userAgent);
let lastEscape = -Infinity;

// Capture phase: these keys never reach the engine's canvas listeners.
window.addEventListener(
  'keydown',
  (e) => {
    const mod = isMac ? e.metaKey : e.ctrlKey;
    const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    if (mod && !e.shiftKey && !e.altKey && (key === 'k' || key === '.')) {
      e.preventDefault();
      e.stopPropagation();
      events.onHostKey?.({ key, mod, shift: false, alt: false });
      return;
    }
    if (key !== 'Escape' || e.ctrlKey || e.metaKey || e.altKey || e.shiftKey || e.repeat) return;
    const now = performance.now();
    if (now - lastEscape > DOUBLE_ESCAPE_MS) {
      lastEscape = now;
      return;
    }
    lastEscape = -Infinity;
    window.parent.focus();
    events.onHostKey?.({ key: 'Escape', mod: false, shift: false, alt: false });
  },
  { capture: true },
);

/** @param {unknown} error */
function isTrap(error) {
  return typeof WebAssembly === 'object' && error instanceof WebAssembly.RuntimeError;
}

window.addEventListener('error', (e) => {
  if (isTrap(e.error)) fail('panic', messageOf(e.error), current);
});
window.addEventListener('unhandledrejection', (e) => {
  if (isTrap(e.reason)) fail('panic', messageOf(e.reason), current);
});

window.addEventListener('pagehide', (e) => {
  if (e.persisted || handle === null) return;
  const gl = canvas.getContext('webgl2') ?? canvas.getContext('webgl');
  gl?.getExtension('WEBGL_lose_context')?.loseContext();
});

window.quireBoardFrame = quireBoardFrame;
