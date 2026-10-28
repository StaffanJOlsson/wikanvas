// @ts-check
// Quire's board frame page (docs/ARCHITECTURE.md §10.4), served as /inkboard/{build}/frame.html + frame.js.
//
// PLACEHOLDER (W0b): there is no engine yet, so `connect` reports and rejects {kind: 'unsupported'} and the host
// falls back to BoardPreview (§9.5). W5 replaces this file with the real frame: feature checks, the counting wasm
// download (`import init from './inkboard.js'`), mount on #board, the visibility wait, host keys, the panic listeners
// and WEBGL_lose_context on pagehide. This placeholder imports nothing at run time, because a dist without the engine
// has no inkboard.js; the type imports below are erased.

/** @import { BoardFrameApi, FrameEvents, QuireBoardFrame } from '../src/board/bridge.ts' */
/** @import { InkboardEmbed } from './inkboard.js' */

const message = 'The whiteboard engine is not part of this build (placeholder frame page).';

/** @type {QuireBoardFrame} */
const quireBoardFrame = {
  /**
   * @param {FrameEvents} events
   * @returns {Promise<BoardFrameApi>}
   */
  connect(events) {
    /** @type {InkboardEmbed | undefined} */
    const engine = globalThis.inkboardEmbed;
    const detail = engine ? `${message} Found engine ${engine.version}.` : message;
    events?.onError?.({ itemId: null, kind: 'unsupported', message: detail });
    return Promise.reject({ kind: 'unsupported', message: detail });
  },
};

window.quireBoardFrame = quireBoardFrame;
