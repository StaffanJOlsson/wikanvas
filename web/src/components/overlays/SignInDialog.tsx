// The sign-in dialog (docs/ARCHITECTURE.md §9.2): a 401 anywhere sends `auth-lost`, and this dialog signs in again
// without leaving the page, so unsaved work stays where it is.
import { useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import { Link, useLocation } from 'react-router';
import { isApiError } from '../../api/client';
import type { Me } from '../../api/gen/Me';
import { afterSignIn, login, meKey } from '../../api/queries/auth';
import { closeSignIn } from '../../state/dialogs';
import { Dialog } from '../ui/Dialog';

export function signInError(e: unknown): string {
  if (isApiError(e, 'invalid_credentials')) return 'Wrong email or password.';
  if (isApiError(e, 'rate_limited')) return 'Too many attempts. Wait a minute and try again.';
  if (isApiError(e, 'network')) return 'The server cannot be reached.';
  return e instanceof Error ? e.message : 'Sign-in failed.';
}

export function SignInDialog() {
  const qc = useQueryClient();
  const location = useLocation();
  const known = qc.getQueryData<Me>(meKey)?.user.email ?? '';
  const [email, setEmail] = useState(known);
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await login({ email, password, remember: true });
      closeSignIn();
      await afterSignIn(qc);
    } catch (err) {
      setError(signInError(err));
    } finally {
      setBusy(false);
    }
  };

  const next = encodeURIComponent(location.pathname + location.search + location.hash);
  return (
    <Dialog title="Sign in again" onClose={closeSignIn} className="sign-in-dialog">
      <form onSubmit={(e) => void submit(e)} className="form">
        <p>Your session ended. Sign in to keep working; nothing on this page is lost.</p>
        <label className="field">
          <span>Email</span>
          <input type="email" autoComplete="username" required value={email} onChange={(e) => setEmail(e.target.value)} />
        </label>
        <label className="field">
          <span>Password</span>
          <input
            type="password"
            autoComplete="current-password"
            required
            value={password}
            data-autofocus={known ? true : undefined}
            onChange={(e) => setPassword(e.target.value)}
          />
        </label>
        {error && <p className="form-error" role="alert">{error}</p>}
        <div className="form-actions">
          <Link to={`/login?next=${next}`} onClick={closeSignIn}>Go to the sign-in page</Link>
          <button type="submit" className="button primary" disabled={busy}>Sign in</button>
        </div>
      </form>
    </Dialog>
  );
}
