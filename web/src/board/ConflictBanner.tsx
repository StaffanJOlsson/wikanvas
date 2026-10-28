// A save answered 412 (docs/ARCHITECTURE.md §9.5): the board changed elsewhere. "Load latest" opens the server's
// board (the local changes are dropped); "Keep mine as a copy" saves the local board as a new sibling board.
export interface ConflictBannerProps {
  onLoadLatest(): void;
  onKeepCopy(): void;
  /** An action is running. */
  busy?: boolean;
  /** Why the last action failed, if it did. */
  error?: string | null;
}

export function ConflictBanner({ onLoadLatest, onKeepCopy, busy = false, error = null }: ConflictBannerProps) {
  return (
    <div role="alert" className="board-banner board-conflict">
      <span>This board was changed somewhere else, so your latest changes were not saved.</span>
      <button type="button" disabled={busy} onClick={onLoadLatest}>
        Load latest
      </button>
      <button type="button" disabled={busy} onClick={onKeepCopy}>
        Keep mine as a copy
      </button>
      {error && <span className="board-conflict-error">{error}</span>}
    </div>
  );
}
