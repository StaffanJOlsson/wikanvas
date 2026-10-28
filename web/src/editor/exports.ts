// The document's export formats in the item menu (docs/ARCHITECTURE.md §9.6.4, D49): Markdown is a download of the
// server's `GET …/doc/markdown?download=true` (one Markdown implementation, in Rust; none in the client), HTML is
// produced here from the editor.
import type { Editor } from '@tiptap/react';
import { markdownDownloadUrl } from '../api/queries/docs';
import type { ExtraExport } from '../components/item/ItemMenu';
import { slugify } from '../lib/urls';

/** Starts a download of `href` (the server names the file). */
export function download(href: string, filename?: string): void {
  const a = document.createElement('a');
  a.href = href;
  a.download = filename ?? '';
  a.rel = 'noopener';
  document.body.appendChild(a);
  a.click();
  a.remove();
}

const escapeHtml = (s: string) => s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c] as string);

/** A standalone HTML document of the editor's content. */
export function htmlDocument(title: string, body: string): string {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>${escapeHtml(title || 'Untitled')}</title>
<style>
body { font: 16px/26px system-ui, sans-serif; max-width: 720px; margin: 48px auto; padding: 0 24px; color: #1d2430; }
pre { background: #f6f7f9; padding: 12px; overflow-x: auto; } code { font-family: ui-monospace, monospace; }
blockquote { border-left: 3px solid #e4e7ec; margin-left: 0; padding-left: 16px; color: #5f6878; }
table { border-collapse: collapse; } td, th { border: 1px solid #e4e7ec; padding: 4px 8px; }
.callout { border-radius: 8px; padding: 8px 16px; background: #eef3ff; } mark { background: #fff0a0; }
</style>
</head>
<body>
<h1>${escapeHtml(title || 'Untitled')}</h1>
${body}
</body>
</html>
`;
}

/** The exports the doc page adds to its ItemMenu. */
export function docExports(itemId: string, title: () => string, editor: () => Editor | null): ExtraExport[] {
  return [
    { label: 'Markdown', run: () => download(markdownDownloadUrl(itemId)) },
    {
      label: 'HTML',
      run: () => {
        const e = editor();
        if (!e) return;
        const blob = new Blob([htmlDocument(title(), e.getHTML())], { type: 'text/html' });
        const url = URL.createObjectURL(blob);
        download(url, `${slugify(title())}.html`);
        setTimeout(() => URL.revokeObjectURL(url), 10_000);
      },
    },
  ];
}
