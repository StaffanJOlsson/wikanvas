// Settings → Appearance (docs/ARCHITECTURE.md §9.6.6): the theme, saved on the account (PATCH /auth/me) and locally
// for theme-init.js.
import type { Theme } from '../../api/gen/Theme';
import { usePatchMe } from '../../api/queries/auth';
import { errorToast } from '../../state/toasts';
import { setTheme, useUi } from '../../state/ui';

const OPTIONS: { theme: Theme; label: string; hint: string }[] = [
  { theme: 'light', label: 'Light', hint: 'Always light' },
  { theme: 'dark', label: 'Dark', hint: 'Always dark' },
  { theme: 'system', label: 'System', hint: 'Follow the operating system' },
];

export function Appearance() {
  const theme = useUi((s) => s.theme);
  const patch = usePatchMe();
  const choose = (t: Theme) => {
    setTheme(t);
    patch.mutate({ theme: t }, { onError: (e) => errorToast(e, 'The theme was not saved') });
  };
  return (
    <section className="settings-section" aria-labelledby="theme-h">
      <h2 id="theme-h">Theme</h2>
      <div role="radiogroup" aria-labelledby="theme-h" className="theme-options">
        {OPTIONS.map((o) => (
          <label key={o.theme} className="radio-card">
            <input type="radio" name="theme" checked={theme === o.theme} onChange={() => choose(o.theme)} />
            <span className="radio-label">{o.label}</span>
            <span className="radio-hint">{o.hint}</span>
          </label>
        ))}
      </div>
    </section>
  );
}
