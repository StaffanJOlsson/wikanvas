// The confirmation dialog behind `confirmAction()` (state/dialogs).
import { settleConfirm, useDialogs } from '../../state/dialogs';
import { Dialog } from '../ui/Dialog';

export function ConfirmDialog() {
  const c = useDialogs((s) => s.confirm);
  if (!c) return null;
  return (
    <Dialog
      title={c.title}
      role="alertdialog"
      className="confirm-dialog"
      onClose={() => settleConfirm(false)}
      footer={
        <>
          <button type="button" className="button" onClick={() => settleConfirm(false)}>Cancel</button>
          <button
            type="button"
            className={c.danger ? 'button danger' : 'button primary'}
            data-autofocus
            onClick={() => settleConfirm(true)}
          >
            {c.confirmLabel ?? 'OK'}
          </button>
        </>
      }
    >
      {c.message && <p>{c.message}</p>}
    </Dialog>
  );
}
