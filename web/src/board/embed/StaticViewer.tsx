// A static card's content (docs/ARCHITECTURE.md §18.5 "Static cards", D64): a read-only TipTap viewer of the
// document's ProseMirror JSON, with the live editor's schema and node views, so its DOM and CSS are the editor's and
// entering editing does not move the text. It is not an editor of the document: no Y.Doc, no session, no menus, and
// it never writes. Only the first MAX_STATIC_BLOCKS top-level blocks are shown. `injectCSS: false`: TipTap's own
// <style> would break the SPA CSP (§11); its base CSS is in editor.css.
import { EditorContent, useEditor, type JSONContent } from '@tiptap/react';
import { useEffect, useMemo } from 'react';
import { viewerExtensions } from '../../editor/extensions';
import { forgetEditor, loadLowlight } from '../../editor/lowlight';
import '../../editor/editor.css';
import { MAX_STATIC_BLOCKS } from './layout';

/** The document's first `max` top-level blocks. */
export function firstBlocks(doc: JSONContent, max = MAX_STATIC_BLOCKS): JSONContent {
  return { type: 'doc', content: (doc.content ?? []).slice(0, max) };
}

/** Blank the way `isBlankDoc` means it (§9.4): no block, or one paragraph without content. */
export function isBlankJson(doc: JSONContent | null | undefined): boolean {
  const c = doc?.content ?? [];
  if (c.length === 0) return true;
  return c.length === 1 && c[0].type === 'paragraph' && (c[0].content?.length ?? 0) === 0;
}

export function StaticViewer({ doc }: { doc: JSONContent }) {
  const content = useMemo(() => firstBlocks(doc), [doc]);
  const editor = useEditor(
    {
      extensions: viewerExtensions(),
      content,
      editable: false,
      immediatelyRender: true,
      shouldRerenderOnTransaction: false,
      injectCSS: false,
      editorProps: { attributes: { class: 'wikanvas-editor' } },
    },
    [content],
  );
  useEffect(() => {
    if (!editor) return;
    loadLowlight(editor);
    return () => forgetEditor(editor);
  }, [editor]);
  return <EditorContent editor={editor} />;
}
