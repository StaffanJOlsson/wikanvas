// Settings → API tokens (docs/ARCHITECTURE.md §7.3, §9.6.6): the list (name, prefix, scopes, created, last used,
// expiry; expired ones greyed with "Expired"), Revoke with a confirmation that names the token, and the "New token"
// dialog. Admin scope or "Never" asks for the password; after creation the secret is shown once with Copy, held only
// in the dialog's state, and dropped when the dialog closes (never in the query cache or in storage, §9.2).
import { useState, type FormEvent } from 'react';
import type { ApiToken } from '../../api/gen/ApiToken';
import type { CreatedToken } from '../../api/gen/CreatedToken';
import type { Scope } from '../../api/gen/Scope';
import { useCreateToken, useRevokeToken, useTokens } from '../../api/queries/tokens';
import { Dialog } from '../../components/ui/Dialog';
import { relativeTime } from '../../lib/time';
import { confirmAction } from '../../state/dialogs';
import { errorMessage, errorToast, toast } from '../../state/toasts';
import { errorField } from '../AuthLayout';

export const SCOPES: { scope: Scope; label: string; hint: string }[] = [
  { scope: 'read', label: 'Read', hint: 'Read documents, boards, collections, search and lists.' },
  { scope: 'write', label: 'Write', hint: 'Also create, edit, move, archive, trash and restore content, stars and uploads.' },
  { scope: 'admin', label: 'Admin', hint: 'Also delete forever, change workspace and account settings, manage sessions and tokens, export and back up.' },
];

export const EXPIRY_CHOICES: { days: number | null; label: string }[] = [
  { days: 7, label: '7 days' },
  { days: 30, label: '30 days' },
  { days: 90, label: '90 days' },
  { days: 365, label: '365 days' },
  { days: null, label: 'Never' },
];

export function isExpired(t: ApiToken, now: number = Date.now()): boolean {
  return t.expires_at !== null && Date.parse(t.expires_at) <= now;
}

function formatDate(t: string): string {
  return new Date(t).toLocaleDateString('en', { year: 'numeric', month: 'short', day: 'numeric' });
}

export function expiryText(t: ApiToken, now: number = Date.now()): string {
  if (t.expires_at === null) return 'Never expires';
  return isExpired(t, now) ? 'Expired' : `Expires ${formatDate(t.expires_at)}`;
}

function highest(scopes: readonly Scope[]): Scope {
  return scopes.includes('admin') ? 'admin' : scopes.includes('write') ? 'write' : 'read';
}

function NewTokenDialog({ onClose }: { onClose(): void }) {
  const { create, isPending } = useCreateToken();
  const [name, setName] = useState('');
  const [scope, setScope] = useState<Scope>('read');
  const [days, setDays] = useState<number | null>(90);
  const [password, setPassword] = useState('');
  const [error, setError] = useState<{ field: string | null; message: string } | null>(null);
  // The only copy of the secret: component state, gone when the dialog unmounts.
  const [created, setCreated] = useState<CreatedToken | null>(null);
  const [copied, setCopied] = useState(false);
  const needsPassword = scope === 'admin' || days === null;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    try {
      const res = await create({
        name: name.trim(),
        scopes: [scope],
        expires_in_days: days,
        ...(needsPassword ? { password } : {}),
      });
      setPassword('');
      setCreated(res);
    } catch (err) {
      setError({ field: errorField(err), message: errorMessage(err, 'The token was not created') });
    }
  };

  const copy = async () => {
    if (!created) return;
    try {
      await navigator.clipboard.writeText(created.token);
      setCopied(true);
    } catch {
      toast({ message: 'Copying failed: select the token and copy it by hand', kind: 'error' });
    }
  };

  if (created) {
    return (
      <Dialog
        title="Your new token"
        onClose={onClose}
        className="token-dialog"
        footer={<button type="button" className="button primary" onClick={onClose}>Done</button>}
      >
        <p><strong>{created.api_token.name}</strong> was created. You won't see this token again: copy it now.</p>
        <div className="secret-row">
          <input
            className="input secret"
            aria-label="Token"
            readOnly
            value={created.token}
            onFocus={(e) => e.currentTarget.select()}
          />
          <button type="button" className="button" data-autofocus onClick={() => void copy()}>
            {copied ? 'Copied' : 'Copy'}
          </button>
        </div>
        <CliNote />
      </Dialog>
    );
  }

  const fieldError = (f: string) => (error?.field === f
    ? <span className="field-error" role="alert">{error.message}</span>
    : null);

  return (
    <Dialog title="New API token" onClose={onClose} className="token-dialog">
      <form className="form" onSubmit={(e) => void submit(e)}>
        <label className="field">
          <span>Name</span>
          <input required maxLength={100} data-autofocus placeholder="e.g. Laptop CLI" value={name} onChange={(e) => setName(e.target.value)} />
          {fieldError('name')}
        </label>
        <fieldset className="field">
          <legend>Scope</legend>
          {SCOPES.map((s) => (
            <label key={s.scope} className="radio-card">
              <input type="radio" name="scope" checked={scope === s.scope} onChange={() => setScope(s.scope)} />
              <span className="radio-label">{s.label}</span>
              <span className="radio-hint">{s.hint}</span>
            </label>
          ))}
          {fieldError('scopes')}
        </fieldset>
        <label className="field">
          <span>Expires after</span>
          <select
            value={days === null ? 'never' : String(days)}
            onChange={(e) => setDays(e.target.value === 'never' ? null : Number(e.target.value))}
          >
            {EXPIRY_CHOICES.map((c) => (
              <option key={c.label} value={c.days === null ? 'never' : String(c.days)}>{c.label}</option>
            ))}
          </select>
          {fieldError('expires_in_days')}
        </label>
        {days === null && (
          <p className="notice warning" role="note">
            A token that never expires stays valid until you revoke it. Prefer an expiry, and revoke tokens you no longer use.
          </p>
        )}
        {needsPassword && (
          <label className="field">
            <span>Your password</span>
            <input type="password" required autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} />
            <span className="field-hint">Needed for {scope === 'admin' ? 'an admin token' : 'a token that never expires'}.</span>
            {fieldError('password')}
          </label>
        )}
        {error && !['name', 'scopes', 'expires_in_days', 'password'].includes(error.field ?? '') && (
          <p className="form-error" role="alert">{error.message}</p>
        )}
        <div className="form-actions">
          <button type="button" className="button" onClick={onClose}>Cancel</button>
          <button type="submit" className="button primary" disabled={isPending}>Create token</button>
        </div>
      </form>
    </Dialog>
  );
}

function CliNote() {
  const url = typeof window !== 'undefined' ? window.location.origin : '<public URL>';
  return (
    <p className="form-note cli-note">
      To use a token with the CLI without putting it in a command line or the shell history, run{' '}
      <code>quire auth login --url {url} --token-stdin</code>, then paste the token and press Ctrl-D.
    </p>
  );
}

export function ApiTokens() {
  const tokens = useTokens();
  const revoke = useRevokeToken();
  const [creating, setCreating] = useState(false);
  const list = tokens.data ?? [];

  const onRevoke = async (t: ApiToken) => {
    const ok = await confirmAction({
      title: `Revoke “${t.name}”?`,
      message: 'Programs using this token stop working at once. Tokens it created are revoked too.',
      confirmLabel: 'Revoke',
      danger: true,
    });
    if (!ok) return;
    revoke.mutate(t, {
      onSuccess: () => toast({ message: `Revoked “${t.name}”`, kind: 'success' }),
      onError: (e) => errorToast(e, 'The token was not revoked'),
    });
  };

  return (
    <section className="settings-section" aria-labelledby="tokens-h">
      <div className="section-head">
        <h2 id="tokens-h">Personal access tokens</h2>
        <button type="button" className="button primary" onClick={() => setCreating(true)}>New token</button>
      </div>
      <p>Tokens let scripts, agents and the <code>quire</code> CLI use the API as you, with the scope you choose.</p>
      {tokens.isError && <p className="form-error" role="alert">{tokens.error.message}</p>}
      {tokens.isSuccess && list.length === 0 && <p className="list-empty">No tokens yet.</p>}
      {list.length > 0 && (
        <table className="token-table">
          <thead>
            <tr>
              <th scope="col">Name</th>
              <th scope="col">Token</th>
              <th scope="col">Scopes</th>
              <th scope="col">Created</th>
              <th scope="col">Last used</th>
              <th scope="col">Expiry</th>
              <th scope="col"><span className="visually-hidden">Actions</span></th>
            </tr>
          </thead>
          <tbody>
            {list.map((t) => (
              <tr key={t.id} className={isExpired(t) ? 'expired' : undefined}>
                <td>{t.name}</td>
                <td><code>{t.prefix}…</code></td>
                <td>
                  {t.scopes.map((s) => (
                    <span key={s} className={`scope-chip${s === highest(t.scopes) ? ' top' : ''}`}>{s}</span>
                  ))}
                </td>
                <td>{relativeTime(t.created_at)}</td>
                <td>{t.last_used_at ? relativeTime(t.last_used_at) : 'Never used'}</td>
                <td>{expiryText(t)}</td>
                <td>
                  <button type="button" className="button small danger" aria-label={`Revoke ${t.name}`} onClick={() => void onRevoke(t)}>
                    Revoke
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <CliNote />
      {creating && <NewTokenDialog onClose={() => setCreating(false)} />}
    </section>
  );
}
