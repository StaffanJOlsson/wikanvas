// The access helpers (D152): what `write` and `manage` allow, the note on a member whose real access differs from the
// membership, and the embed cards' editable targets.
import { describe, expect, it } from 'vitest';
import { isEditableTarget } from '../board/embed/controller';
import { makeCollection, makeDetail, makeDirectoryUser, makeItem, makeMember } from '../test/mockApi';
import { canManage, canWrite, effectiveNote } from './access';

describe('access helpers', () => {
  it('write and manage write; only manage manages', () => {
    expect(['none', 'read', 'write', 'manage', undefined].map((a) => canWrite(a as never))).toEqual([false, false, true, true, false]);
    expect(['none', 'read', 'write', 'manage', undefined].map((a) => canManage(a as never))).toEqual([false, false, false, true, false]);
  });

  it('notes what really applies to a member', () => {
    const viewer = makeDirectoryUser({ id: 'v', role: 'viewer' });
    const member = makeDirectoryUser({ id: 'm', role: 'member' });
    const admin = makeDirectoryUser({ id: 'a', role: 'admin' });
    expect(effectiveNote(makeMember(viewer, { access: 'write', effective_access: 'read' }))).toBe('reads only (viewer)');
    expect(effectiveNote(makeMember(member, { access: 'read', effective_access: 'write' }))).toBe('can edit (default access)');
    expect(effectiveNote(makeMember(admin, { access: 'read', effective_access: 'manage' }))).toBe('manages (admin)');
    expect(effectiveNote(makeMember(member, { access: 'write' }))).toBeNull();
    expect(effectiveNote(makeMember(member, { access: 'write', effective_access: 'none', disabled: true }))).toBeNull();
  });

  it('a board card edits only a document of a collection the caller may change', () => {
    const doc = makeItem({ id: 'd1', collection_id: 'c1' });
    expect(isEditableTarget(makeDetail(doc, makeCollection({ id: 'c1', my_access: 'write' })))).toBe(true);
    expect(isEditableTarget(makeDetail(doc, makeCollection({ id: 'c1', my_access: 'read' })))).toBe(false);
  });
});
