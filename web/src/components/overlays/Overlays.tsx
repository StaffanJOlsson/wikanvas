// Every global overlay (docs/ARCHITECTURE.md §9.3): QuickSwitcher, ShortcutsDialog, MoveDialog, CollectionDialog,
// ConfirmDialog, SignInDialog and Toasts. AppShell renders it once; a 401 anywhere (`auth-lost`) opens the sign-in
// dialog.
import { useEffect } from 'react';
import { AUTH_LOST_EVENT } from '../../api/client';
import { openSignIn, useDialogs } from '../../state/dialogs';
import { Toasts } from '../toasts/Toasts';
import { CollectionDialog } from './CollectionDialog';
import { ConfirmDialog } from './ConfirmDialog';
import { MoveDialog } from './MoveDialog';
import { QuickSwitcher } from '../quickSwitcher/QuickSwitcher';
import { ShortcutsDialog } from './ShortcutsDialog';
import { SignInDialog } from './SignInDialog';

export function Overlays() {
  const active = useDialogs((s) => s.active);
  const signIn = useDialogs((s) => s.signIn);

  useEffect(() => {
    const onLost = () => openSignIn();
    window.addEventListener(AUTH_LOST_EVENT, onLost);
    return () => window.removeEventListener(AUTH_LOST_EVENT, onLost);
  }, []);

  return (
    <>
      {active?.kind === 'quick-switcher' && <QuickSwitcher />}
      {active?.kind === 'shortcuts' && <ShortcutsDialog />}
      {active?.kind === 'move' && <MoveDialog key={active.item.id} item={active.item} />}
      {active?.kind === 'collection' && (
        <CollectionDialog key={active.collection?.id ?? 'new'} collection={active.collection} />
      )}
      <ConfirmDialog />
      {signIn && <SignInDialog />}
      <Toasts />
    </>
  );
}
