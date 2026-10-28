// The keyboard shortcuts dialog (`?` outside text fields; docs/ARCHITECTURE.md §9.6.3). Mod+Shift+P is not listed:
// there is no publish shortcut in M1 (D23).
import { closeDialog } from '../../state/dialogs';
import { Dialog } from '../ui/Dialog';

export function modKeyLabel(): string {
  return typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.userAgent) ? '⌘' : 'Ctrl';
}

export function ShortcutsDialog() {
  const mod = modKeyLabel();
  const groups: [string, [string, string][]][] = [
    ['Everywhere', [
      [`${mod}+K`, 'Quick switcher'],
      [`${mod}+.`, 'Show or hide the sidebar'],
      [`${mod}+S`, 'Save now'],
      ['?', 'Keyboard shortcuts'],
    ]],
    ['Sidebar tree', [
      ['↑ / ↓', 'Previous / next row'],
      ['→ / ←', 'Expand / collapse, or go to the first child / the parent'],
      ['Home / End', 'First / last row'],
      ['Enter', 'Open'],
      [`${mod}+Enter`, 'Open in a new tab'],
      ['F2', 'Rename'],
      ['Alt+Shift+→ / ←', 'Indent / outdent'],
      ['Alt+Shift+↑ / ↓', 'Move up / down (manual sort)'],
      ['Delete', 'Move to Trash'],
      ['Type letters', 'Jump to a title'],
      ['Esc', 'Back to the page'],
    ]],
    ['"+" menu', [
      ['D', 'New document'],
      ['B', 'New board'],
    ]],
  ];
  return (
    <Dialog title="Keyboard shortcuts" onClose={closeDialog} className="shortcuts-dialog">
      {groups.map(([title, rows]) => (
        <section key={title}>
          <h3>{title}</h3>
          <dl className="shortcut-list">
            {rows.map(([k, what]) => (
              <div key={k} className="shortcut">
                <dt><kbd>{k}</kbd></dt>
                <dd>{what}</dd>
              </div>
            ))}
          </dl>
        </section>
      ))}
    </Dialog>
  );
}
