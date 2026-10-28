// The top bar's save indicator, fed by state/saveStatus (docs/ARCHITECTURE.md §9.3).
import { saveStateLabel, useSaveStatus } from '../../state/saveStatus';

export function SaveStatus({ itemId }: { itemId: string }) {
  const s = useSaveStatus(itemId);
  const label = s.message ?? saveStateLabel(s.state);
  return (
    <span className="save-status" role="status" data-state={s.state}>
      {label}
      {s.retry && (s.state === 'error' || s.state === 'offline') && (
        <>
          {' '}
          <button type="button" className="icon-button" onClick={s.retry}>
            Retry
          </button>
        </>
      )}
    </span>
  );
}
