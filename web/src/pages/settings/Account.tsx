// Settings → Account (docs/ARCHITECTURE.md §9.6.6): name and email, password, and sessions. With `?force=1` (a
// generated owner password, §7.8) the password form comes first and must be used before anything else.
import { useEffect, useState, type FormEvent } from 'react';
import { useNavigate, useSearchParams } from 'react-router';
import { useChangePassword, useMe, usePatchMe, useRevokeSession, useSessions } from '../../api/queries/auth';
import { relativeTime } from '../../lib/time';
import { confirmAction } from '../../state/dialogs';
import { errorMessage, errorToast, toast } from '../../state/toasts';
import { MIN_PASSWORD } from '../Setup';

function Profile() {
  const me = useMe();
  const patch = usePatchMe();
  const [name, setName] = useState('');
  const [email, setEmail] = useState('');
  const user = me.data?.user;
  useEffect(() => {
    if (user) {
      setName(user.name);
      setEmail(user.email);
    }
  }, [user]);
  if (!user) return null;
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const body: { name?: string; email?: string } = {};
    if (name.trim() !== user.name) body.name = name.trim();
    if (email.trim() !== user.email) body.email = email.trim();
    if (Object.keys(body).length === 0) return;
    patch.mutate(body, { onSuccess: () => toast({ message: 'Saved', kind: 'success' }) });
  };
  return (
    <section className="settings-section" aria-labelledby="profile-h">
      <h2 id="profile-h">Profile</h2>
      <form className="form" onSubmit={submit}>
        <label className="field">
          <span>Name</span>
          <input required autoComplete="name" value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="field">
          <span>Email</span>
          <input type="email" required autoComplete="email" value={email} onChange={(e) => setEmail(e.target.value)} />
        </label>
        {patch.error && <p className="form-error" role="alert">{errorMessage(patch.error)}</p>}
        <div className="form-actions"><button type="submit" className="button primary" disabled={patch.isPending}>Save</button></div>
      </form>
    </section>
  );
}

function Password({ force }: { force: boolean }) {
  const change = useChangePassword();
  const navigate = useNavigate();
  const [current, setCurrent] = useState('');
  const [next, setNext] = useState('');
  const [repeat, setRepeat] = useState('');
  const [error, setError] = useState<string | null>(null);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (new TextEncoder().encode(next).length < MIN_PASSWORD) return setError(`Use at least ${MIN_PASSWORD} characters.`);
    if (next !== repeat) return setError('The new passwords do not match.');
    setError(null);
    change.mutate({ current, new: next }, {
      onSuccess: () => {
        setCurrent('');
        setNext('');
        setRepeat('');
        toast({ message: 'Password changed. Your other sessions were signed out.', kind: 'success' });
        if (force) void navigate('/home', { replace: true });
      },
      onError: (err) => setError(errorMessage(err, 'The password was not changed')),
    });
  };
  return (
    <section className={`settings-section${force ? ' highlighted' : ''}`} aria-labelledby="password-h">
      <h2 id="password-h">Password</h2>
      {force && (
        <p className="notice" role="alert">
          This account still has a generated password. Choose your own password to continue.
        </p>
      )}
      <form className="form" onSubmit={submit}>
        <label className="field">
          <span>Current password</span>
          <input type="password" required autoComplete="current-password" autoFocus={force} value={current} onChange={(e) => setCurrent(e.target.value)} />
        </label>
        <label className="field">
          <span>New password</span>
          <input type="password" required autoComplete="new-password" value={next} onChange={(e) => setNext(e.target.value)} />
        </label>
        <label className="field">
          <span>Repeat the new password</span>
          <input type="password" required autoComplete="new-password" value={repeat} onChange={(e) => setRepeat(e.target.value)} />
        </label>
        {error && <p className="form-error" role="alert">{error}</p>}
        <div className="form-actions"><button type="submit" className="button primary" disabled={change.isPending}>Change password</button></div>
      </form>
    </section>
  );
}

function Sessions() {
  const sessions = useSessions();
  const revoke = useRevokeSession();
  const list = sessions.data ?? [];
  const others = list.filter((s) => !s.current);
  return (
    <section className="settings-section" aria-labelledby="sessions-h">
      <h2 id="sessions-h">Sessions</h2>
      {sessions.isError && <p className="form-error" role="alert">{sessions.error.message}</p>}
      <ul className="session-list">
        {list.map((s) => (
          <li key={s.id} className="session-row">
            <span className="session-agent">{s.user_agent || 'Unknown browser'}{s.current && <strong> · This browser</strong>}</span>
            <span className="session-meta">
              {s.ip ?? 'unknown address'} · signed in {relativeTime(s.created_at)} · last seen {relativeTime(s.last_seen_at)}
            </span>
            {!s.current && (
              <button type="button" className="button small" onClick={() => revoke.mutate(s.id, { onError: (e) => errorToast(e) })}>
                Sign out
              </button>
            )}
          </li>
        ))}
      </ul>
      {others.length > 0 && (
        <button
          type="button"
          className="button"
          onClick={() => void confirmAction({
            title: 'Sign out every other session?',
            message: `${others.length === 1 ? '1 other session' : `${others.length} other sessions`} will have to sign in again. API tokens keep working.`,
            confirmLabel: 'Sign out others',
            danger: true,
          }).then((ok) => ok && revoke.mutate('others', { onError: (e) => errorToast(e) }))}
        >
          Sign out all other sessions
        </button>
      )}
    </section>
  );
}

export function Account() {
  const me = useMe();
  const [params] = useSearchParams();
  const force = params.get('force') === '1' || me.data?.must_change_password === true;
  return (
    <>
      {force && <Password force />}
      <Profile />
      {!force && <Password force={false} />}
      <Sessions />
    </>
  );
}
