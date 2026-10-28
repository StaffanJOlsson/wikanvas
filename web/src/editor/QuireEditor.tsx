// QuireEditor (docs/ARCHITECTURE.md §6.1, §9.3, §9.6.5): TipTap on the doc session's Y.Doc, with its menus. The
// Y.Doc already holds the server state (applied with origin 'server' before the editor exists); content is never
// passed to the editor.
import { EditorContent, useEditor, type Editor } from '@tiptap/react';
import { useEffect, useMemo, useRef } from 'react';
import { headingsOfNode, type Heading } from './contents';
import type { DocSession } from './docSession';
import { editorExtensions, uploadImages } from './extensions';
import { forgetEditor, loadLowlight } from './lowlight';
import { BlockHandle, SelectionMenu, SuggestPopup, TableMenu } from './menus';
import type { SlashContext } from './slash';
import { Popup } from './suggest';
import './editor.css';

export interface QuireEditorProps {
  itemId: string;
  session: DocSession;
  readOnly: boolean;
  /** Called with the document's headings whenever they change. */
  onHeadings?(headings: Heading[]): void;
  /** The editor, once it exists (and null when it goes). */
  onEditor?(editor: Editor | null): void;
  /** Board actions of the slash menu (the page knows the item). */
  pickBoard(editor: Editor): void;
  newBoard(editor: Editor): void;
}

function sameHeadings(a: Heading[], b: Heading[]): boolean {
  return a.length === b.length && a.every((h, i) => h.blockId === b[i].blockId && h.text === b[i].text && h.level === b[i].level);
}

export function QuireEditor({ itemId, session, readOnly, onHeadings, onEditor, pickBoard, newBoard }: QuireEditorProps) {
  const popups = useMemo(() => ({ slash: new Popup(), mention: new Popup() }), []);
  const callbacks = useRef({ pickBoard, newBoard, onHeadings });
  callbacks.current = { pickBoard, newBoard, onHeadings };
  const fileInput = useRef<HTMLInputElement>(null);
  const editorRef = useRef<Editor | null>(null);
  const ctx: SlashContext = useMemo(
    () => ({
      pickImage: () => fileInput.current?.click(),
      pickBoard: (e) => callbacks.current.pickBoard(e),
      newBoard: (e) => callbacks.current.newBoard(e),
    }),
    [],
  );
  const lastHeadings = useRef<Heading[]>([]);
  const editor = useEditor(
    {
      extensions: editorExtensions({ itemId, ydoc: session.ydoc, slash: popups.slash, mention: popups.mention, ctx }),
      editable: !readOnly,
      immediatelyRender: true,
      shouldRerenderOnTransaction: false,
      // TipTap's base CSS is in editor.css: its inline <style> would break the SPA CSP (§11).
      injectCSS: false,
      editorProps: {
        attributes: { class: 'quire-editor', 'aria-label': 'Document body', spellcheck: 'true' },
      },
      onCreate: ({ editor: e }) => {
        const h = headingsOfNode(e.state.doc);
        lastHeadings.current = h;
        callbacks.current.onHeadings?.(h);
      },
      onUpdate: ({ editor: e }) => {
        const h = headingsOfNode(e.state.doc);
        if (!sameHeadings(h, lastHeadings.current)) {
          lastHeadings.current = h;
          callbacks.current.onHeadings?.(h);
        }
      },
    },
    [session],
  );

  useEffect(() => {
    if (!editor) return;
    editorRef.current = editor;
    loadLowlight(editor);
    session.setPmJsonProvider(() => editor.getJSON());
    onEditor?.(editor);
    return () => {
      forgetEditor(editor);
      session.setPmJsonProvider(null);
      onEditor?.(null);
    };
    // onEditor is a page callback; the editor is what matters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editor, session]);

  useEffect(() => {
    editor?.setEditable(!readOnly);
  }, [editor, readOnly]);

  return (
    <div className="quire-editor-shell" data-readonly={readOnly || undefined}>
      {editor && !readOnly && (
        <>
          <SelectionMenu editor={editor} />
          <TableMenu editor={editor} />
          <BlockHandle editor={editor} />
        </>
      )}
      <EditorContent editor={editor} />
      <SuggestPopup popup={popups.slash} label="Insert a block" />
      <SuggestPopup popup={popups.mention} label="Mention a page" />
      <input
        ref={fileInput}
        type="file"
        accept="image/*"
        multiple
        hidden
        onChange={(e) => {
          const files = [...(e.target.files ?? [])];
          const ed = editorRef.current;
          if (ed && files.length > 0) uploadImages(ed, itemId, files, ed.state.selection.from);
          e.target.value = '';
        }}
      />
    </div>
  );
}
