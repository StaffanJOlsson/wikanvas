// Settings → Members (docs/ARCHITECTURE.md §9.6.6, D148), for the owner and admins: the people with their role (a
// select where the caller may change it), Disable and Enable, "Invite people" (role and expiry; the link is shown once
// with Copy, held only in the dialog's state and dropped when it closes, like a token's secret, §9.2), and the pending
// invites with Revoke. The rules are the server's (only the owner makes or unmakes an admin; nobody changes the owner
// or themselves); the page only offers what they allow. Until per-collection permissions, everyone sees every
// collection, which the page says.
import { useState, type FormEvent } from 'react';
import type { Invite } from '../../api/gen/Invite';
import type { Role } from '../../api/gen/Role';
import type { UserInfo } from '../../api/gen/UserInfo';
import { useMe } from '../../api/queries/auth';
import { useCreateInvite, useInvites, usePatchUser, useRevokeInvite, useUsers } from '../../api/queries/users';
import { Dialog } from '../../components/ui/Dialog';
import { relativeTime } from '../../lib/time';
import { confirmAction } from '../../state/dialogs';
import { errorMessage, errorToast, toast } from '../../state/toasts';
import { errorField } from '../AuthLayout';

export const ROLE_LABELS: Record<Role, string> = {
  owner: 'Owner',
  admin: 'Admin',
  member: 'Member',
  viewer: 'Viewer',
};

export const ROLE_HINTS: Record<Exclude<Role, 'owner'>, string> = {
  admin: 'Everything a member can, and manage members, invites, the workspace and the trash.',
  member: 'Read, write and organize every collection.',
  viewer: 'Read every collection.',
};

export const INVITE_EXPIRY: { days: number; label: string }[] = [
  { days: 1, label: '1 day' },
  { days: 7, label: '7 days' },
  { days: 14, label: '14 days' },
  { days: 30, label: '30 days' },
];

/** Whether the owner and admins see Settings → Members. */
export function managesMembers(role: Role | undefined): boolean {
  return role === 'owner' || role === 'admin';
}

/** The roles `me` may give an account of `target` (empty: it cannot change it at all). */
export function assignableRoles(me: { id: string; role: Role }, target: UserInfo): Role[] {
  if (target.role === 'owner' || target.id === me.id) return [];
  if (me.role === 'owner') return ['admin', 'member', 'viewer'];
  if (me.role === 'admin' && target.role !== 'admin') return ['member', 'viewer'];
  return [];
}

function formatDate(t: string): string {
  return new Date(t).toLocaleDateString('en', { year: 'numeric', month: 'short', day: 'numeric' });
}

function InviteDialog({ myRole, onClose }: { myRole: Role; onClose(): void }) {
  const { create, isPending } = useCreateInvite();
  const roles: Exclude<Role, 'owner'>[] = myRole === 'owner' ? ['member', 'viewer', 'admin'] : ['member', 'viewer'];
  const [role, setRole] = useState<Exclude<Role, 'owner'>>('member');
  const [days, setDays] = useState(7);
  const [note, setNote] = useState('');
  const [error, setError] = useState<{ field: string | null; message: string } | null>(null);
  // The only copy of the link: component state, gone when the dialog unmounts.
  const [url, setUrl] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    try {
      const trimmed = note.trim();
      const res = await create({ role, expires_in_days: days, ...(trimmed ? { note: trimmed } : {}) });
      setUrl(res.url);
    } catch (err) {
      setError({ field: errorField(err), message: errorMessage(err, 'The invite was not created') });
    }
  };

  const copy = async () => {
    if (!url) return;
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
    } catch {
      toast({ message: 'Copying failed: select the link and copy it by hand', kind: 'error' });
    }
  };

  if (url) {
    return (
      <Dialog
        title="Invite link"
        onClose={onClose}
        className="token-dialog"
        footer={<button type="button" className="button primary" onClick={onClose}>Done</button>}
      >
        <p>
          Send this link to the person you invite. It works once, as a <strong>{ROLE_LABELS[role].toLowerCase()}</strong>,
          for {days === 1 ? '1 day' : `${days} days`}. You won't see it again: copy it now.
        </p>
        <div className="secret-row">
          <input
            className="input secret"
            aria-label="Invite link"
            readOnly
            value={url}
            onFocus={(e) => e.currentTarget.select()}
          />
          <button type="button" className="button" data-autofocus onClick={() => void copy()}>
            {copied ? 'Copied' : 'Copy'}
          </button>
        </div>
      </Dialog>
    );
  }

  const fieldError = (f: string) => (error?.field === f
    ? <span className="field-error" role="alert">{error.message}</span>
    : null);

  return (
    <Dialog title="Invite people" onClose={onClose} className="token-dialog">
      <form className="form" onSubmit={(e) => void submit(e)}>
        <fieldset className="field">
          <legend>Role</legend>
          {roles.map((r) => (
            <label key={r} className="radio-card">
              <input type="radio" name="role" checked={role === r} onChange={() => setRole(r)} />
              <span className="radio-label">{ROLE_LABELS[r]}</span>
              <span className="radio-hint">{ROLE_HINTS[r]}</span>
            </label>
          ))}
          {fieldError('role')}
        </fieldset>
        <label className="field">
          <span>Link expires after</span>
          <select value={String(days)} onChange={(e) => setDays(Number(e.target.value))}>
            {INVITE_EXPIRY.map((c) => <option key={c.days} value={String(c.days)}>{c.label}</option>)}
          </select>
          {fieldError('expires_in_days')}
        </label>
        <label className="field">
          <span>Note (optional)</span>
          <input maxLength={200} placeholder="e.g. For Alex" value={note} onChange={(e) => setNote(e.target.value)} />
          <span className="field-hint">Only the owner and admins see it.</span>
          {fieldError('note')}
        </label>
        {error && !['role', 'expires_in_days', 'note'].includes(error.field ?? '') && (
          <p className="form-error" role="alert">{error.message}</p>
        )}
        <div className="form-actions">
          <button type="button" className="button" onClick={onClose}>Cancel</button>
          <button type="submit" className="button primary" data-autofocus disabled={isPending}>Create link</button>
        </div>
      </form>
    </Dialog>
  );
}

function PeopleTable({ me, users }: { me: { id: string; role: Role }; users: UserInfo[] }) {
  const patch = usePatchUser();

  const setRole = (u: UserInfo, role: Role) => {
    patch.mutate({ id: u.id, patch: { role } }, {
      onSuccess: () => toast({ message: `${u.name} is now ${ROLE_LABELS[role].toLowerCase()}`, kind: 'success' }),
      onError: (e) => errorToast(e, 'The role was not changed'),
    });
  };

  const toggle = async (u: UserInfo) => {
    const disabling = u.disabled_at === null;
    if (disabling) {
      const ok = await confirmAction({
        title: `Disable ${u.name}?`,
        message: `${u.email} is signed out everywhere at once, and their API tokens and pending invites stop working. You can enable the account again; it keeps everything it wrote.`,
        confirmLabel: 'Disable',
        danger: true,
      });
      if (!ok) return;
    }
    patch.mutate({ id: u.id, patch: { disabled: disabling } }, {
      onSuccess: () => toast({ message: disabling ? `Disabled ${u.name}` : `Enabled ${u.name}`, kind: 'success' }),
      onError: (e) => errorToast(e, disabling ? 'The account was not disabled' : 'The account was not enabled'),
    });
  };

  return (
    <table className="token-table members-table">
      <thead>
        <tr>
          <th scope="col">Name</th>
          <th scope="col">Email</th>
          <th scope="col">Role</th>
          <th scope="col">Joined</th>
          <th scope="col"><span className="visually-hidden">Actions</span></th>
        </tr>
      </thead>
      <tbody>
        {users.map((u) => {
          const roles = assignableRoles(me, u);
          const disabled = u.disabled_at !== null;
          return (
            <tr key={u.id} className={disabled ? 'disabled' : undefined}>
              <td>
                {u.name}
                {u.id === me.id && <span className="field-hint"> (you)</span>}
                {disabled && <span className="scope-chip member-state">Disabled</span>}
              </td>
              <td>{u.email}</td>
              <td>
                {roles.length > 0 ? (
                  <select
                    aria-label={`Role of ${u.name}`}
                    value={u.role}
                    disabled={patch.isPending}
                    onChange={(e) => setRole(u, e.target.value as Role)}
                  >
                    {roles.map((r) => <option key={r} value={r}>{ROLE_LABELS[r]}</option>)}
                  </select>
                ) : ROLE_LABELS[u.role]}
              </td>
              <td>{relativeTime(u.created_at)}</td>
              <td>
                {roles.length > 0 && (
                  <button
                    type="button"
                    className={`button small${disabled ? '' : ' danger'}`}
                    aria-label={`${disabled ? 'Enable' : 'Disable'} ${u.name}`}
                    disabled={patch.isPending}
                    onClick={() => void toggle(u)}
                  >
                    {disabled ? 'Enable' : 'Disable'}
                  </button>
                )}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function PendingInvites({ users }: { users: UserInfo[] }) {
  const invites = useInvites();
  const revoke = useRevokeInvite();
  const list = invites.data ?? [];
  const nameOf = (id: string | null) => users.find((u) => u.id === id)?.name ?? '—';

  const onRevoke = async (i: Invite) => {
    const ok = await confirmAction({
      title: `Revoke this ${ROLE_LABELS[i.role].toLowerCase()} invite?`,
      message: i.note ? `“${i.note}”: the link stops working at once.` : 'The link stops working at once.',
      confirmLabel: 'Revoke',
      danger: true,
    });
    if (!ok) return;
    revoke.mutate(i, {
      onSuccess: () => toast({ message: 'Invite revoked', kind: 'success' }),
      onError: (e) => errorToast(e, 'The invite was not revoked'),
    });
  };

  return (
    <section className="settings-section" aria-labelledby="invites-h">
      <h2 id="invites-h">Pending invites</h2>
      {invites.isError && <p className="form-error" role="alert">{invites.error.message}</p>}
      {invites.isSuccess && list.length === 0 && <p className="list-empty">No pending invites.</p>}
      {list.length > 0 && (
        <table className="token-table invites-table">
          <thead>
            <tr>
              <th scope="col">Role</th>
              <th scope="col">Note</th>
              <th scope="col">Invited by</th>
              <th scope="col">Expires</th>
              <th scope="col"><span className="visually-hidden">Actions</span></th>
            </tr>
          </thead>
          <tbody>
            {list.map((i) => (
              <tr key={i.id}>
                <td>{ROLE_LABELS[i.role]}</td>
                <td>{i.note ?? '—'}</td>
                <td>{nameOf(i.created_by)}</td>
                <td>{formatDate(i.expires_at)}</td>
                <td>
                  <button
                    type="button"
                    className="button small danger"
                    aria-label={`Revoke the ${ROLE_LABELS[i.role].toLowerCase()} invite${i.note ? ` ${i.note}` : ''}`}
                    onClick={() => void onRevoke(i)}
                  >
                    Revoke
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

export function Members() {
  const me = useMe();
  const role = me.data?.user.role;
  const allowed = managesMembers(role);
  const users = useUsers(allowed);
  const [inviting, setInviting] = useState(false);
  if (!me.data) return null;
  if (!allowed) {
    return <p className="page-note">Only the owner and admins manage the workspace's members.</p>;
  }
  const self = { id: me.data.user.id, role: me.data.user.role };
  const list = users.data ?? [];
  return (
    <>
      <section className="settings-section" aria-labelledby="members-h">
        <div className="section-head">
          <h2 id="members-h">People</h2>
          <button type="button" className="button primary" onClick={() => setInviting(true)}>Invite people</button>
        </div>
        <p>
          People join with an invite link. Members read and write every collection, viewers read every collection;
          admins also manage members and invites. Access per collection comes later.
        </p>
        {users.isError && <p className="form-error" role="alert">{users.error.message}</p>}
        {list.length > 0 && <PeopleTable me={self} users={list} />}
      </section>
      <PendingInvites users={list} />
      {inviting && <InviteDialog myRole={self.role} onClose={() => setInviting(false)} />}
    </>
  );
}
