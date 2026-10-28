// The toast stack (docs/ARCHITECTURE.md §9.3): "Moved to <parent> · Undo" and error messages.
import { X } from 'lucide-react';
import { dismissToast, useToasts } from '../../state/toasts';

export function Toasts() {
  const toasts = useToasts((s) => s.toasts);
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast toast-${t.kind}`} role={t.kind === 'error' ? 'alert' : 'status'}>
          <span className="toast-message">{t.message}</span>
          {t.action && (
            <>
              <span className="toast-dot" aria-hidden>·</span>
              <button
                type="button"
                className="toast-action"
                onClick={() => {
                  const action = t.action!;
                  dismissToast(t.id);
                  void Promise.resolve(action.run());
                }}
              >
                {t.action.label}
              </button>
            </>
          )}
          <button type="button" className="icon-button toast-close" aria-label="Dismiss" onClick={() => dismissToast(t.id)}>
            <X size={14} aria-hidden />
          </button>
        </div>
      ))}
    </div>
  );
}
