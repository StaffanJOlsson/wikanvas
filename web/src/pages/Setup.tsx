// First-run setup (docs/ARCHITECTURE.md §8, §9.1): the setup code from the server log, the workspace name and the
// owner account. `POST /setup` creates the owner, seeds "Getting started" and signs in.
import { useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import { Navigate, useNavigate } from 'react-router';
import { isApiError } from '../api/client';
import { afterSignIn, setup, useSetupStatus } from '../api/queries/auth';
import { errorMessage } from '../state/toasts';
import { AuthLayout, errorField } from './AuthLayout';

export const MIN_PASSWORD = 12;

export default function Setup() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const status = useSetupStatus();
  const [form, setForm] = useState({ code: '', workspace_name: '', name: '', email: '', password: '', confirm: '' });
  const [error, setError] = useState<{ field: string | null; message: string } | null>(null);
  const [busy, setBusy] = useState(false);

  if (status.data && !status.data.needs_setup) return <Navigate to="/login" replace />;

  const set = (k: keyof typeof form) => (e: { target: { value: string } }) => setForm((f) => ({ ...f, [k]: e.target.value }));

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if ([...new TextEncoder().encode(form.password)].length < MIN_PASSWORD) {
      setError({ field: 'password', message: `Use at least ${MIN_PASSWORD} characters.` });
      return;
    }
    if (form.password !== form.confirm) {
      setError({ field: 'confirm', message: 'The passwords do not match.' });
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await setup({
        code: form.code.trim(),
        workspace_name: form.workspace_name.trim(),
        name: form.name.trim(),
        email: form.email.trim(),
        password: form.password,
      });
      await afterSignIn(qc);
      void navigate('/home', { replace: true });
    } catch (err) {
      setBusy(false);
      if (isApiError(err, 'already_setup')) {
        void navigate('/login', { replace: true });
        return;
      }
      setError({ field: errorField(err), message: errorMessage(err, 'Setup failed') });
    }
  };

  const fieldError = (f: string) => (error?.field === f ? <span className="field-error" role="alert">{error.message}</span> : null);

  return (
    <AuthLayout title="Set up Quire">
      <form className="form" onSubmit={(e) => void submit(e)}>
        <p className="form-note">
          The setup code is in the server log (“First-run setup: … code …”), or in <code>QUIRE_SETUP_TOKEN</code>.
        </p>
        <label className="field">
          <span>Setup code</span>
          <input required autoFocus autoComplete="one-time-code" placeholder="XXXX-XXXX-XXXX" value={form.code} onChange={set('code')} />
          {fieldError('code')}
        </label>
        <label className="field">
          <span>Workspace name</span>
          <input required value={form.workspace_name} onChange={set('workspace_name')} />
          {fieldError('workspace_name')}
        </label>
        <label className="field">
          <span>Your name</span>
          <input required autoComplete="name" value={form.name} onChange={set('name')} />
          {fieldError('name')}
        </label>
        <label className="field">
          <span>Email</span>
          <input type="email" required autoComplete="username" value={form.email} onChange={set('email')} />
          {fieldError('email')}
        </label>
        <label className="field">
          <span>Password</span>
          <input type="password" required autoComplete="new-password" value={form.password} onChange={set('password')} />
          {fieldError('password')}
        </label>
        <label className="field">
          <span>Repeat the password</span>
          <input type="password" required autoComplete="new-password" value={form.confirm} onChange={set('confirm')} />
          {fieldError('confirm')}
        </label>
        {error && error.field === null && <p className="form-error" role="alert">{error.message}</p>}
        <button type="submit" className="button primary wide" disabled={busy}>Create workspace</button>
      </form>
    </AuthLayout>
  );
}
