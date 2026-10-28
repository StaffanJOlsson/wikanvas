// Sign in (docs/ARCHITECTURE.md §8, §9.1): email, password and "Keep me signed in", then back to `?next=`.
import { useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import { Navigate, useNavigate, useSearchParams } from 'react-router';
import { afterSignIn, login } from '../api/queries/auth';
import { useConfig } from '../api/queries/config';
import { signInError } from '../components/overlays/SignInDialog';
import { safeNext } from '../lib/urls';
import { AuthLayout } from './AuthLayout';

export default function Login() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const config = useConfig();
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [remember, setRemember] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const next = safeNext(params.get('next'));

  if (config.data?.setup_required) return <Navigate to="/setup" replace />;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await login({ email: email.trim(), password, remember });
      await afterSignIn(qc);
      void navigate(next, { replace: true });
    } catch (err) {
      setError(signInError(err));
      setBusy(false);
    }
  };

  return (
    <AuthLayout title="Sign in">
      <form className="form" onSubmit={(e) => void submit(e)}>
        <label className="field">
          <span>Email</span>
          <input type="email" autoComplete="username" required autoFocus value={email} onChange={(e) => setEmail(e.target.value)} />
        </label>
        <label className="field">
          <span>Password</span>
          <input type="password" autoComplete="current-password" required value={password} onChange={(e) => setPassword(e.target.value)} />
        </label>
        <label className="checkbox">
          <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
          <span>Keep me signed in</span>
        </label>
        {error && <p className="form-error" role="alert">{error}</p>}
        <button type="submit" className="button primary wide" disabled={busy}>Sign in</button>
      </form>
    </AuthLayout>
  );
}
