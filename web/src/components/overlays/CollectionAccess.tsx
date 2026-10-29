// The collection dialog's Access section (docs/ARCHITECTURE.md §8 "Access per collection", D152), for whoever manages
// the collection: the default access (saved with the dialog's Save or Create), the people with a membership (their
// access select saves at once, Remove deletes it), "Add a person" from the workspace directory, and the note that the
// owner and admins manage every collection. Creating a collection offers only the default access.
import { useId, useState } from 'react';
import type { Collection } from '../../api/gen/Collection';
import type { CollectionMember } from '../../api/gen/CollectionMember';
import type { DefaultAccess } from '../../api/gen/DefaultAccess';
import type { MemberAccess } from '../../api/gen/MemberAccess';
import { useMe } from '../../api/queries/auth';
import { useDirectory, useMembers, useRemoveMember, useSetMember } from '../../api/queries/access';
import {
  DEFAULT_ACCESS_OPTIONS,
  effectiveNote,
  MEMBER_ACCESS_OPTIONS,
  ROLE_LABELS,
} from '../../lib/access';
import { errorToast, toast } from '../../state/toasts';

export interface AccessSectionProps {
  /** Null while creating: only the default access is offered. */
  collection: Collection | null;
  defaultAccess: DefaultAccess;
  onDefaultAccess(access: DefaultAccess): void;
}

export function AccessSection({ collection, defaultAccess, onDefaultAccess }: AccessSectionProps) {
  const hintId = useId();
  const hint = DEFAULT_ACCESS_OPTIONS.find((o) => o.value === defaultAccess)?.hint;
  return (
    <fieldset className="field access-section">
      <legend>Access</legend>
      <label className="field">
        <span>Default access</span>
        <select
          aria-label="Default access"
          value={defaultAccess}
          aria-describedby={hintId}
          onChange={(e) => onDefaultAccess(e.target.value as DefaultAccess)}
        >
          {DEFAULT_ACCESS_OPTIONS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
        </select>
      </label>
      <p className="field-hint" id={hintId}>{hint}</p>
      {collection ? <People collection={collection} /> : (
        <p className="field-hint">You manage the new collection and can add people to it once it exists.</p>
      )}
      <p className="field-hint access-note">The owner and admins manage every collection.</p>
    </fieldset>
  );
}

function People({ collection }: { collection: Collection }) {
  const peopleId = useId();
  const me = useMe().data?.user.id;
  const members = useMembers(collection.id);
  const setMember = useSetMember();
  const removeMember = useRemoveMember();
  const list = members.data ?? [];

  const change = (m: CollectionMember, access: MemberAccess) => {
    setMember.mutate({ collectionId: collection.id, userId: m.user.id, access }, {
      onError: (e) => errorToast(e, `The access of ${m.user.name} was not changed`),
    });
  };

  const remove = (m: CollectionMember) => {
    removeMember.mutate({ collectionId: collection.id, userId: m.user.id }, {
      onSuccess: () => toast({ message: `Removed ${m.user.name} from ${collection.name}`, kind: 'success' }),
      onError: (e) => errorToast(e, `${m.user.name} was not removed`),
    });
  };

  return (
    <div className="access-people">
      <h3 className="access-subhead" id={peopleId}>People</h3>
      {members.isError && <p className="form-error" role="alert">{members.error.message}</p>}
      {members.isSuccess && list.length === 0 && <p className="field-hint">Nobody is added yet.</p>}
      {list.length > 0 && (
        <table className="token-table access-table" aria-labelledby={peopleId}>
          <tbody>
            {list.map((m) => {
              const note = effectiveNote(m);
              return (
                <tr key={m.user.id} className={m.disabled ? 'disabled' : undefined}>
                  <td>
                    <span className="access-name">
                      {m.user.name}
                      {m.user.id === me && <span className="field-hint"> (you)</span>}
                      {m.disabled && <span className="field-hint"> (disabled)</span>}
                    </span>
                    <span className="field-hint access-role">
                      {ROLE_LABELS[m.user.role]}
                      {note && ` · ${note}`}
                    </span>
                  </td>
                  <td>
                    <select
                      aria-label={`Access of ${m.user.name}`}
                      value={m.access}
                      disabled={setMember.isPending}
                      onChange={(e) => change(m, e.target.value as MemberAccess)}
                    >
                      {MEMBER_ACCESS_OPTIONS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
                    </select>
                  </td>
                  <td>
                    <button
                      type="button"
                      className="button small"
                      aria-label={`Remove ${m.user.name}`}
                      disabled={removeMember.isPending}
                      onClick={() => remove(m)}
                    >
                      Remove
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      <AddPerson collection={collection} members={list} />
      <p className="field-hint">Changes to people apply at once.</p>
    </div>
  );
}

function AddPerson({ collection, members }: { collection: Collection; members: CollectionMember[] }) {
  const headId = useId();
  const directory = useDirectory();
  const setMember = useSetMember();
  const [userId, setUserId] = useState('');
  const [access, setAccess] = useState<MemberAccess>('write');
  const taken = new Set(members.map((m) => m.user.id));
  // The owner and admins manage every collection already.
  const candidates = (directory.data ?? []).filter((u) => !taken.has(u.id) && u.role !== 'owner' && u.role !== 'admin');
  const chosen = candidates.find((u) => u.id === userId);

  const add = () => {
    if (!chosen) return;
    setMember.mutate({ collectionId: collection.id, userId: chosen.id, access }, {
      onSuccess: () => {
        setUserId('');
        toast({ message: `Added ${chosen.name} to ${collection.name}`, kind: 'success' });
      },
      onError: (e) => errorToast(e, `${chosen.name} was not added`),
    });
  };

  return (
    <div className="access-add" role="group" aria-labelledby={headId}>
      <h3 className="access-subhead" id={headId}>Add a person</h3>
      {directory.isError && <p className="form-error" role="alert">{directory.error.message}</p>}
      <div className="access-add-row">
        <select aria-label="Person to add" value={chosen ? userId : ''} onChange={(e) => setUserId(e.target.value)}>
          <option value="">Choose a person…</option>
          {candidates.map((u) => <option key={u.id} value={u.id}>{u.name} ({ROLE_LABELS[u.role]})</option>)}
        </select>
        <select aria-label="Access to give" value={access} onChange={(e) => setAccess(e.target.value as MemberAccess)}>
          {MEMBER_ACCESS_OPTIONS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
        </select>
        <button type="button" className="button" disabled={!chosen || setMember.isPending} onClick={add}>Add</button>
      </div>
      {directory.isSuccess && candidates.length === 0 && (
        <p className="field-hint">There is nobody else to add.</p>
      )}
    </div>
  );
}
