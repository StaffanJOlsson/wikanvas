// Settings → Workspace (docs/ARCHITECTURE.md §9.6.6): the name and the logo. The logo is uploaded as raw bytes
// (`application/octet-stream`, §7.1) and then set with `PATCH /workspace {logo_attachment_id}`.
import { useEffect, useRef, useState, type FormEvent } from 'react';
import { useMe } from '../../api/queries/auth';
import { uploadAttachment, usePatchWorkspace } from '../../api/queries/workspace';
import { WorkspaceLogo } from '../../components/sidebar/WorkspaceHeader';
import { errorMessage, errorToast, toast } from '../../state/toasts';

export function WorkspaceSettings() {
  const me = useMe();
  const patch = usePatchWorkspace();
  const [name, setName] = useState('');
  const [uploading, setUploading] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  const ws = me.data?.workspace;
  useEffect(() => {
    if (ws) setName(ws.name);
  }, [ws]);
  if (!me.data || !ws) return null;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!name.trim() || name.trim() === ws.name) return;
    patch.mutate({ name: name.trim() }, { onSuccess: () => toast({ message: 'Saved', kind: 'success' }) });
  };

  const upload = async (f: File) => {
    setUploading(true);
    try {
      const att = await uploadAttachment(f, f.name);
      await patch.mutateAsync({ logo_attachment_id: att.id });
    } catch (e) {
      errorToast(e, 'The logo was not uploaded');
    } finally {
      setUploading(false);
      if (file.current) file.current.value = '';
    }
  };

  return (
    <>
      <section className="settings-section" aria-labelledby="ws-name-h">
        <h2 id="ws-name-h">Name</h2>
        <form className="form" onSubmit={submit}>
          <label className="field">
            <span>Workspace name</span>
            <input required maxLength={100} value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          {patch.error && <p className="form-error" role="alert">{errorMessage(patch.error)}</p>}
          <div className="form-actions"><button type="submit" className="button primary" disabled={patch.isPending}>Save</button></div>
        </form>
      </section>
      <section className="settings-section" aria-labelledby="ws-logo-h">
        <h2 id="ws-logo-h">Logo</h2>
        <div className="logo-row">
          <WorkspaceLogo me={me.data} size={48} />
          <label className="button">
            {uploading ? 'Uploading…' : 'Upload a logo'}
            <input
              ref={file}
              type="file"
              accept="image/png,image/jpeg,image/gif,image/webp,image/avif"
              className="visually-hidden"
              disabled={uploading}
              onChange={(e) => {
                const f = e.target.files?.[0];
                if (f) void upload(f);
              }}
            />
          </label>
          {ws.logo_url && (
            <button type="button" className="button" onClick={() => patch.mutate({ logo_attachment_id: null }, { onError: (e) => errorToast(e) })}>
              Remove
            </button>
          )}
        </div>
        <p className="form-note">A square PNG, JPEG, GIF, WebP or AVIF image works best. Without a logo the sidebar shows the first letter of the name.</p>
      </section>
    </>
  );
}
