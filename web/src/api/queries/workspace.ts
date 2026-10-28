// The workspace name and logo (docs/ARCHITECTURE.md §7.2), and attachment uploads, which send raw bytes as
// `application/octet-stream` (§7.1: the server sniffs the type).
import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseMutationResult,
  type UseQueryResult,
} from '@tanstack/react-query';
import { api, type ApiError } from '../client';
import type { Attachment } from '../gen/Attachment';
import type { Me } from '../gen/Me';
import type { Workspace } from '../gen/Workspace';
import type { WorkspacePatch } from '../gen/WorkspacePatch';
import { meKey } from './auth';
import { withQuery } from './lists';

export const workspaceKey = ['workspace'] as const;

export function useWorkspace(): UseQueryResult<Workspace, ApiError> {
  return useQuery<Workspace, ApiError>({
    queryKey: workspaceKey,
    queryFn: ({ signal }) => api<Workspace>('GET', '/workspace', { signal }),
  });
}

/** `PATCH /workspace` (admin scope). `logo_attachment_id: null` removes the logo. Updates `me.workspace` too. */
export function usePatchWorkspace(): UseMutationResult<Workspace, ApiError, WorkspacePatch> {
  const qc = useQueryClient();
  return useMutation<Workspace, ApiError, WorkspacePatch>({
    mutationFn: (patch) => api<Workspace>('PATCH', '/workspace', { json: patch }),
    onSuccess: (ws) => {
      qc.setQueryData(workspaceKey, ws);
      qc.setQueryData<Me>(meKey, (me) => (me ? { ...me, workspace: ws } : me));
    },
  });
}

/** `POST /attachments?filename=…[&item_id=…]` with the raw bytes. */
export function uploadAttachment(file: Blob, filename: string, itemId?: string): Promise<Attachment> {
  return api<Attachment>('POST', withQuery('/attachments', { filename, item_id: itemId }), {
    body: file,
    headers: { 'Content-Type': 'application/octet-stream' },
  });
}
