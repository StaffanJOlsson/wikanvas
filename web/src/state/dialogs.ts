// Which global overlay is open (docs/ARCHITECTURE.md §9.3): the quick switcher, the shortcuts dialog, the Move dialog
// and the collection dialog (one at a time), plus the confirmation and sign-in dialogs, which can open over them.
// components/overlays/Overlays renders them.
import { create } from 'zustand';
import type { Collection } from '../api/gen/Collection';
import type { ItemSummary } from '../api/gen/ItemSummary';

export type ActiveDialog =
  | { kind: 'quick-switcher' }
  | { kind: 'shortcuts' }
  | { kind: 'move'; item: ItemSummary }
  /** `collection: null` creates one. */
  | { kind: 'collection'; collection: Collection | null };

export interface ConfirmOptions {
  title: string;
  message?: string;
  confirmLabel?: string;
  /** The confirm button is styled as destructive. */
  danger?: boolean;
}

interface ConfirmState extends ConfirmOptions {
  resolve(ok: boolean): void;
}

interface DialogState {
  active: ActiveDialog | null;
  confirm: ConfirmState | null;
  signIn: boolean;
}

export const useDialogs = create<DialogState>(() => ({ active: null, confirm: null, signIn: false }));

export function openDialog(d: ActiveDialog): void {
  useDialogs.setState({ active: d });
}

export function closeDialog(): void {
  useDialogs.setState({ active: null });
}

/** Opens the quick switcher, or closes it when it is open (Mod+K). */
export function toggleQuickSwitcher(): void {
  const cur = useDialogs.getState().active;
  useDialogs.setState({ active: cur?.kind === 'quick-switcher' ? null : { kind: 'quick-switcher' } });
}

/** Asks for confirmation; resolves true when confirmed, false when cancelled (or replaced by another question). */
export function confirmAction(opts: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    useDialogs.getState().confirm?.resolve(false);
    useDialogs.setState({ confirm: { ...opts, resolve } });
  });
}

export function settleConfirm(ok: boolean): void {
  const c = useDialogs.getState().confirm;
  useDialogs.setState({ confirm: null });
  c?.resolve(ok);
}

export function openSignIn(): void {
  useDialogs.setState({ signIn: true });
}

export function closeSignIn(): void {
  useDialogs.setState({ signIn: false });
}

/** Closes everything (tests, sign-out). */
export function resetDialogs(): void {
  useDialogs.getState().confirm?.resolve(false);
  useDialogs.setState({ active: null, confirm: null, signIn: false });
}
