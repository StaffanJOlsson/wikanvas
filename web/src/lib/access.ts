// Access per collection in the web client (docs/ARCHITECTURE.md §8 "Access per collection", D152). The server decides
// and filters (every list only holds collections the caller may read); the client only hides what it would refuse:
// `read` opens and stars, `write` creates, edits, moves, trashes and restores items, `manage` also changes the
// collection itself (its settings, default access, members, place, trashing and restoring it).
import type { CollectionAccess } from '../api/gen/CollectionAccess';
import type { CollectionMember } from '../api/gen/CollectionMember';
import type { DefaultAccess } from '../api/gen/DefaultAccess';
import type { ItemDetail } from '../api/gen/ItemDetail';
import type { MemberAccess } from '../api/gen/MemberAccess';
import type { Role } from '../api/gen/Role';

/** May create, edit, move, trash and restore the collection's items. */
export function canWrite(access: CollectionAccess | null | undefined): boolean {
  return access === 'write' || access === 'manage';
}

/** May change the collection itself: settings, default access, members, place, trash and restore. */
export function canManage(access: CollectionAccess | null | undefined): boolean {
  return access === 'manage';
}

/** The item's collection lets the caller change it (trash and archive make an item read-only on their own). */
export function itemWritable(detail: Pick<ItemDetail, 'collection'>): boolean {
  return canWrite(detail.collection.my_access);
}

/** The banner of a document or board in a collection the caller may only read. */
export const READ_ONLY_COLLECTION = 'You can read this collection but not change it.';

export const ROLE_LABELS: Record<Role, string> = {
  owner: 'Owner',
  admin: 'Admin',
  member: 'Member',
  viewer: 'Viewer',
};

export const DEFAULT_ACCESS_OPTIONS: readonly { value: DefaultAccess; label: string; hint: string }[] = [
  {
    value: 'none',
    label: 'Only members',
    hint: 'Only the people added here can open it, besides the owner and admins.',
  },
  {
    value: 'read',
    label: 'Everyone can read',
    hint: 'Everyone in the workspace can read it; the people added here can do more.',
  },
  {
    value: 'write',
    label: 'Members edit, viewers read',
    hint: 'Members can edit it and viewers can read it; the people added here can do more.',
  },
];

export const MEMBER_ACCESS_OPTIONS: readonly { value: MemberAccess; label: string }[] = [
  { value: 'read', label: 'Read' },
  { value: 'write', label: 'Write' },
  { value: 'manage', label: 'Manage' },
];

const ACCESS_VERBS: Record<CollectionAccess, string> = {
  none: 'no access',
  read: 'reads only',
  write: 'can edit',
  manage: 'manages',
};

/**
 * What a member may really do, when it differs from their membership: "reads only (viewer)" for a viewer given more
 * than read, "can edit (default access)" when the default gives more, "manages (admin)" for the owner and admins.
 * Null when the membership is what applies, and for a disabled account (it has none until it is enabled again).
 */
export function effectiveNote(m: CollectionMember): string | null {
  if (m.disabled || m.effective_access === m.access) return null;
  const role = m.user.role;
  const why = role === 'viewer' || role === 'owner' || role === 'admin' ? ROLE_LABELS[role].toLowerCase() : 'default access';
  return `${ACCESS_VERBS[m.effective_access]} (${why})`;
}
