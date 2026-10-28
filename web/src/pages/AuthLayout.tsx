// The frame of the setup and sign-in pages: a centered card outside the app shell.
import type { ReactNode } from 'react';

export function AuthLayout({ title, children }: { title: string; children: ReactNode }) {
  return (
    <main className="auth-page" id="main" tabIndex={-1}>
      <div className="auth-card">
        <div className="auth-brand" aria-hidden>
          <img src="/favicon.svg" alt="" width={32} height={32} />
          <span>Quire</span>
        </div>
        <h1>{title}</h1>
        {children}
      </div>
    </main>
  );
}

/** The field a 422 `validation` error names, if any. */
export function errorField(e: unknown): string | null {
  const d = (e as { details?: unknown } | null)?.details;
  if (d && typeof d === 'object' && 'field' in d && typeof (d as { field: unknown }).field === 'string') {
    return (d as { field: string }).field;
  }
  return null;
}
