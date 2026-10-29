// The sidebar's workspace header (docs/ARCHITECTURE.md §9.3, §9.6.2): logo (the workspace logo, else a letter tile in
// --accent), workspace name, the user's name, and the ▾ menu: Settings, Theme, Shortcuts, About, Sign out.
import { useQueryClient } from '@tanstack/react-query';
import { ChevronDown } from 'lucide-react';
import { useNavigate } from 'react-router';
import type { Me } from '../../api/gen/Me';
import type { Theme } from '../../api/gen/Theme';
import { logout, usePatchMe } from '../../api/queries/auth';
import { openDialog, resetDialogs } from '../../state/dialogs';
import { clearToasts, errorToast } from '../../state/toasts';
import { setTheme, useUi } from '../../state/ui';
import { Menu, type MenuEntry } from '../ui/Menu';

const themeLabels: Record<Theme, string> = { light: 'Light', dark: 'Dark', system: 'System' };

export function WorkspaceLogo({ me, size = 24 }: { me: Pick<Me, 'workspace'>; size?: number }) {
  const ws = me.workspace;
  if (ws.logo_url) return <img className="workspace-logo" src={ws.logo_url} alt="" width={size} height={size} />;
  const letter = [...ws.name.trim()][0]?.toUpperCase() ?? 'W';
  return <span className="letter-tile workspace-tile" aria-hidden style={{ width: size, height: size }}>{letter}</span>;
}

export function WorkspaceHeader({ me }: { me: Me | undefined }) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const theme = useUi((s) => s.theme);
  const patchMe = usePatchMe().mutateAsync;

  const chooseTheme = (t: Theme) => {
    setTheme(t);
    void patchMe({ theme: t }).catch((e: unknown) => errorToast(e, 'The theme was not saved'));
  };

  const signOut = async () => {
    try {
      await logout();
    } catch {
      // Signed out already, or offline: leave the page either way.
    }
    resetDialogs();
    clearToasts();
    qc.clear();
    void navigate('/login', { replace: true });
  };

  const entries: MenuEntry[] = [
    { label: 'Settings', onSelect: () => void navigate('/settings/account') },
    { heading: 'Theme' },
    ...(['light', 'dark', 'system'] as const).map((t) => ({
      label: themeLabels[t],
      checked: theme === t,
      onSelect: () => chooseTheme(t),
    })),
    { separator: true },
    { label: 'Keyboard shortcuts', onSelect: () => openDialog({ kind: 'shortcuts' }) },
    { label: 'About Wikanvas', onSelect: () => void navigate('/settings/about') },
    { separator: true },
    { label: 'Sign out', onSelect: () => void signOut() },
  ];

  return (
    <div className="workspace-header">
      {me && <WorkspaceLogo me={me} />}
      <div className="workspace-names">
        <span className="workspace-name">{me?.workspace.name ?? 'Wikanvas'}</span>
        <span className="workspace-user">{me?.user.name}</span>
      </div>
      <Menu label="Workspace menu" entries={entries}>
        <ChevronDown size={16} aria-hidden />
      </Menu>
    </div>
  );
}
