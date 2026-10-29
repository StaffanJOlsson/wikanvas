// The board save loop (docs/ARCHITECTURE.md §9.5, D32). `onChange` schedules a save 1.5 s after the last change and
// at most 10 s after the first unsaved one. A save takes the frame's `snapshot()`, writes it to IndexedDB as the
// unsaved snapshot, and PUTs it with `If-Match: "r{rev}"` and `X-Wikanvas-Engine`:
// - 200: `markSaved(snapshot.revision)` (changes made during the PUT stay dirty), the new rev, the snapshot cleared;
// - 412: the loop stops and reports the conflict (ConflictBanner); 413: an error naming the limit;
// - network error: `offline`, retried after 1, 2, 5, 10, then every 30 s, and at once when the browser is online;
// - 401: `signed_out`; anything else: `error` with a retry.
// `flush()` saves now and is what Ctrl/Cmd+S in the canvas (`onSave`), a hidden page, a route change and a board
// switch call. The loop is registered in `pending`, so `beforeunload` warns while the board is dirty. Saves never use
// `fetch keepalive` (a board is too large for its 64 KB).
import { ApiError, isApiError } from '../api/client';
import type { BoardPutResult } from '../api/gen/BoardPutResult';
import { conflictRev, putBoard } from '../api/queries/boards';
import { registerPending } from '../state/pending';
import { setSaveStatus } from '../state/saveStatus';
import type { BoardFrameApi } from './bridge';
import { clearUnsaved, writeUnsaved, type UnsavedSnapshot } from './unsaved';

/** Quiet time after the last change (D32). */
export const QUIET_MS = 1500;
/** Longest time a change waits (D32). */
export const MAX_MS = 10_000;
/** Back-off of a save that failed on the network. */
export const BACKOFF_MS = [1000, 2000, 5000, 10_000, 30_000] as const;
/** A board larger than this shows a warning (D33). */
export const LARGE_BOARD_BYTES = 15 * 1024 * 1024;

/** A 412: the board changed elsewhere. `board` is the snapshot that could not be saved. */
export interface BoardConflict {
  board: string;
  /** The server's current revision, when the 412 named it. */
  rev: number | null;
}

export interface SaveLoopOptions {
  itemId: string;
  /** The revision the frame's board is based on. */
  rev: number;
  /** The connected frame (null while there is none). */
  frame(): BoardFrameApi | null;
  put?: (itemId: string, board: string, base: number, engine?: string) => Promise<BoardPutResult>;
  writeUnsaved?: (itemId: string, s: UnsavedSnapshot) => Promise<void>;
  clearUnsaved?: (itemId: string) => Promise<void>;
  onSaved?(result: BoardPutResult): void;
  onConflict?(conflict: BoardConflict): void;
  /** Called with the snapshot's size when it is larger than LARGE_BOARD_BYTES. */
  onLarge?(bytes: number): void;
  /** The board limit in bytes (`limits.max_board_mb`), for the 413 message. */
  limitBytes?: number;
  now?: () => number;
}

type Timer = ReturnType<typeof setTimeout>;

export class BoardSaveLoop {
  readonly itemId: string;
  /** The revision the next PUT is based on. */
  rev: number;
  private readonly o: SaveLoopOptions;
  private quiet: Timer | null = null;
  private max: Timer | null = null;
  private retry: Timer | null = null;
  private attempt = 0;
  private inFlight: Promise<void> | null = null;
  private again = false;
  /** A forced save (after a restore) failed: the next save sends the board even if the frame is not dirty. */
  private forceNext = false;
  private conflicted = false;
  private disposed = false;
  private detach: (() => void) | null = null;

  constructor(o: SaveLoopOptions) {
    this.o = o;
    this.itemId = o.itemId;
    this.rev = o.rev;
  }

  /** Registers in `pending` and flushes when the page is hidden or the browser comes back online. */
  attach(): void {
    if (this.detach) return;
    const unregister = registerPending(this.itemId, this);
    const onVisibility = () => {
      if (document.visibilityState === 'hidden') void this.flush();
    };
    const onOnline = () => {
      if (this.retry !== null) this.retryNow();
    };
    document.addEventListener('visibilitychange', onVisibility);
    window.addEventListener('online', onOnline);
    this.detach = () => {
      unregister();
      document.removeEventListener('visibilitychange', onVisibility);
      window.removeEventListener('online', onOnline);
    };
  }

  /** The frame reported a change (`onChange`). */
  change(_revision: number): void {
    if (this.disposed || this.conflicted) return;
    if (this.quiet !== null) clearTimeout(this.quiet);
    this.quiet = setTimeout(() => void this.save(), QUIET_MS);
    if (this.max === null) this.max = setTimeout(() => void this.save(), MAX_MS);
  }

  /** Whether something is unsaved: a scheduled or running save, or a dirty board in the frame. */
  hasPending(): boolean {
    return (
      this.quiet !== null ||
      this.max !== null ||
      this.inFlight !== null ||
      this.retry !== null ||
      this.forceNext ||
      this.dirty()
    );
  }

  /** Saves now if anything is unsaved; resolves when that save finished (it reports failures through saveStatus). */
  async flush(): Promise<void> {
    this.clearTimers();
    if (this.inFlight) await this.inFlight;
    if ((this.dirty() || this.forceNext) && !this.conflicted) await this.save();
  }

  /** Saves the frame's board even if it is not dirty (after restoring an unsaved snapshot). */
  saveNow(): Promise<void> {
    return this.save(true);
  }

  /** Whether the last save hit a 412 and the loop waits for the conflict to be resolved. */
  isConflicted(): boolean {
    return this.conflicted;
  }

  /** The conflict was resolved (the latest board was loaded at `rev`): saving resumes from there. */
  resume(rev: number): void {
    this.rev = rev;
    this.conflicted = false;
    this.attempt = 0;
  }

  /** Stops the timers and listeners. A running save still finishes. */
  dispose(): void {
    this.disposed = true;
    this.clearTimers();
    if (this.retry !== null) clearTimeout(this.retry);
    this.retry = null;
    this.detach?.();
    this.detach = null;
  }

  /** The frame, when it shows this board. A frame whose engine trapped (a panic) may throw: it counts as gone. */
  private frame(): BoardFrameApi | null {
    try {
      const f = this.o.frame();
      return f && f.current() === this.itemId ? f : null;
    } catch {
      return null;
    }
  }

  private dirty(): boolean {
    try {
      return this.frame()?.isDirty() ?? false;
    } catch {
      return false;
    }
  }

  private clearTimers(): void {
    if (this.quiet !== null) clearTimeout(this.quiet);
    if (this.max !== null) clearTimeout(this.max);
    this.quiet = null;
    this.max = null;
  }

  private retryNow(): void {
    if (this.retry !== null) clearTimeout(this.retry);
    this.retry = null;
    void this.save();
  }

  private save(force = false): Promise<void> {
    this.clearTimers();
    if (this.retry !== null) {
      clearTimeout(this.retry);
      this.retry = null;
    }
    if (this.inFlight) {
      this.again = true;
      return this.inFlight;
    }
    const frame = this.frame();
    const forced = force || this.forceNext;
    if (!frame || this.conflicted || (!forced && !this.dirty())) return Promise.resolve();
    let snap: ReturnType<BoardFrameApi['snapshot']>;
    try {
      snap = frame.snapshot();
    } catch (e) {
      setSaveStatus(this.itemId, { state: 'error', message: `The board could not be read: ${String(e)}` });
      return Promise.resolve();
    }
    if (snap.itemId !== this.itemId) return Promise.resolve();
    if (snap.board.length > LARGE_BOARD_BYTES) this.o.onLarge?.(snap.board.length);
    this.forceNext = false;
    const run = this.put(frame, snap.board, snap.revision, forced);
    this.inFlight = run;
    void run.finally(() => {
      if (this.inFlight === run) this.inFlight = null;
      if (this.again && !this.disposed) {
        this.again = false;
        if (this.dirty()) void this.save();
      }
    });
    return run;
  }

  private async put(frame: BoardFrameApi, board: string, revision: number, forced: boolean): Promise<void> {
    const id = this.itemId;
    const now = this.o.now ?? Date.now;
    setSaveStatus(id, 'saving');
    await (this.o.writeUnsaved ?? writeUnsaved)(id, { baseRev: this.rev, board, at: now() });
    try {
      const put = this.o.put ?? putBoard;
      const result = await put(id, board, this.rev, frame.engineVersion);
      this.rev = result.rev;
      this.attempt = 0;
      if (frame.current() === id) frame.markSaved(revision);
      await (this.o.clearUnsaved ?? clearUnsaved)(id);
      setSaveStatus(id, 'saved');
      this.o.onSaved?.(result);
    } catch (e) {
      if (forced && !isApiError(e, 'rev_mismatch')) this.forceNext = true;
      this.failed(e, board);
    }
  }

  private failed(e: unknown, board: string): void {
    const id = this.itemId;
    if (isApiError(e, 'rev_mismatch')) {
      this.conflicted = true;
      setSaveStatus(id, { state: 'error', message: 'Changed elsewhere' });
      this.o.onConflict?.({ board, rev: conflictRev(e) });
      return;
    }
    if (isApiError(e, 'payload_too_large')) {
      const limit = (e.details as { limit_bytes?: number } | undefined)?.limit_bytes ?? this.o.limitBytes;
      const mb = limit ? `${Math.round(limit / (1024 * 1024))} MB` : 'the size';
      setSaveStatus(id, { state: 'error', message: `The board is larger than ${mb} and cannot be saved` });
      return;
    }
    if (e instanceof ApiError && e.status === 401) {
      setSaveStatus(id, { state: 'signed_out', retry: () => void this.save(true) });
      return;
    }
    if (e instanceof ApiError && e.code === 'network') {
      const delay = BACKOFF_MS[Math.min(this.attempt, BACKOFF_MS.length - 1)];
      this.attempt += 1;
      setSaveStatus(id, { state: 'offline', retry: () => this.retryNow() });
      if (!this.disposed) this.retry = setTimeout(() => this.retryNow(), delay);
      return;
    }
    const message = e instanceof Error ? e.message : String(e);
    setSaveStatus(id, { state: 'error', message, retry: () => void this.save(true) });
  }
}
