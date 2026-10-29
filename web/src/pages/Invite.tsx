// The invite page `/invite/:token` (docs/ARCHITECTURE.md §9.1, §9.6.6, D148), outside the signed-in shell like setup
// and sign-in: `GET /invites/{token}` says whether the link can be used, for which workspace and role; the form takes
// a name, an email and a password (setup's rules), `POST /invites/{token}/accept` creates the account and signs it
// in, and the page goes to Home. A link that is unknown, used, expired or revoked gets its own message and a way to
// sign in.
import { useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import { Link, useNavigate, useParams } from 'react-router';
import { isApiError } from '../api/client';
import type { InviteState } from '../api/gen/InviteState';
import { afterSignIn } from '../api/queries/auth';
import { acceptInvite, useInviteLookup } from '../api/queries/users';
import { errorMessage } from '../state/toasts';
import { AuthLayout, errorField } from './AuthLayout';
import { MIN_PASSWORD } from './Setup';

/** What the page says about a link that cannot be used. */
export const INVALID_TEXT: Record<Exclude<InviteState, 'pending'> | 'unknown', { title: string; message: string }> = {
  used: { title: 'This invite was used', message: 'An invite link works once, and this one has been used. If it was yours, sign in.' },
  expired: { title: 'This invite has expired', message: 'Ask the person who invited you for a new link.' },
  revoked: { title: 'This invite was revoked', message: 'Ask the person who invited you for a new link.' },
  unknown: { title: 'This invite link is not valid', message: 'Check that you copied the whole link, or ask for a new one.' },
};

const ARTICLE: Record<string, string> = { admin: 'an admin', member: 'a member', viewer: 'a viewer' };

function Invalid({ state }: { state: keyof typeof INVALID_TEXT }) {
  const text = INVALID_TEXT[state];
  return (
    <AuthLayout title={text.title}>
      <p className="form-note" role="alert">{text.message}</p>
      <Link className="button wide" to="/login">Sign in</Link>
    </AuthLayout>
  );
}

export default function InvitePage() {
  const { token = '' } = useParams();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const invite = useInviteLookup(token);
  const [form, setForm] = useState({ name: '', email: '', password: '', confirm: '' });
  const [error, setError] = useState<{ field: string | null; message: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [gone, setGone] = useState(false);

  if (invite.isPending) {
    return (
      <AuthLayout title="Invitation">
        <p className="form-note">Checking the invite link…</p>
      </AuthLayout>
    );
  }
  if (invite.isError) {
    if (isApiError(invite.error, 'not_found')) return <Invalid state="unknown" />;
    return (
      <AuthLayout title="Invitation">
        <p className="form-error" role="alert">{errorMessage(invite.error, 'The invite could not be checked')}</p>
      </AuthLayout>
    );
  }
  const data = invite.data;
  if (gone) return <Invalid state="unknown" />;
  if (data.state !== 'pending') return <Invalid state={data.state} />;

  const set = (k: keyof typeof form) => (e: { target: { value: string } }) => setForm((f) => ({ ...f, [k]: e.target.value }));

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (new TextEncoder().encode(form.password).length < MIN_PASSWORD) {
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
      await acceptInvite(token, { name: form.name.trim(), email: form.email.trim(), password: form.password });
      await afterSignIn(qc);
      void navigate('/home', { replace: true });
    } catch (err) {
      setBusy(false);
      if (isApiError(err, 'not_found')) {
        setGone(true);
        return;
      }
      const field = errorField(err);
      const message = isApiError(err, 'conflict') && field === 'email'
        ? 'Another account has this email. Sign in instead, or use another address.'
        : errorMessage(err, 'The account was not created');
      setError({ field, message });
    }
  };

  const fieldError = (f: string) => (error?.field === f ? <span className="field-error" role="alert">{error.message}</span> : null);

  return (
    <AuthLayout title={`Join ${data.workspace_name}`}>
      <form className="form" onSubmit={(e) => void submit(e)}>
        <p className="form-note">
          You are invited to <strong>{data.workspace_name}</strong> as {ARTICLE[data.role] ?? data.role}. Create your
          account to continue.
        </p>
        <label className="field">
          <span>Your name</span>
          <input required autoFocus autoComplete="name" value={form.name} onChange={set('name')} />
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
        <button type="submit" className="button primary wide" disabled={busy}>Join {data.workspace_name}</button>
      </form>
    </AuthLayout>
  );
}
