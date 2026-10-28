// The body of the W0b placeholder pages.
import type { ReactNode } from 'react';

export function PlaceholderPage({ title, children }: { title?: string; children?: ReactNode }) {
  return (
    <section className="placeholder-page">
      {title && <h1>{title}</h1>}
      {children && <p>{children}</p>}
    </section>
  );
}
