// QuireEditor (docs/ARCHITECTURE.md §6.1, §9.3, §9.6.5): TipTap on the doc session's Y.Doc, with its menus. The
// Y.Doc already holds the server state (applied with origin 'server' before the editor exists); content is never
// passed to the editor. `variant="embed"` is the editor of a board card (§18.5 Editing step 2): its popups go to
// `popupRoot`, it has no block handle and no heading anchors, and Backspace at the start of an empty body goes to
// `titleInput`. Without `pickBoard`/`newBoard` the slash menu leaves out "Embed board…"/"New board".
import { EditorContent, useEditor, type Editor } from '@tiptap/react';
import { useEffect, useMemo, useRef } from 'react';
import { useNavigate } from 'react-router';
import { headingsOfNode, type Heading } from './contents';
import type { DocSession } from './docSession';
import { editorExtensions, openOutside, uploadImages } from './extensions';
import type { LinkTarget } from './links';
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
  /** Board actions of the slash menu (the page knows the item); an item is left out without its callback. */
  pickBoard?(editor: Editor): void;
  newBoard?(editor: Editor): void;
  /** `'embed'`: in a board card (additive, M1.1). Default `'page'`. */
  variant?: 'page' | 'embed';
  /** Where the embed variant's popups and bubble menus go (the overlay's `.board-embed-popups`). */
  popupRoot?: HTMLElement | null;
  /** The title input Backspace at the start of an empty body moves to (default: the page's `#item-title`). */
  titleInput?(): HTMLElement | null;
}

function sameHeadings(a: Heading[], b: Heading[]): boolean {
  return a.length === b.length && a.every((h, i) => h.blockId === b[i].blockId && h.text === b[i].text && h.level === b[i].level);
}

export function QuireEditor(props: QuireEditorProps) {
  const { itemId, session, readOnly, onHeadings, onEditor, pickBoard, newBoard, variant = 'page', popupRoot, titleInput } = props;
  const embed = variant === 'embed';
  const portal = embed ? (popupRoot ?? null) : null;
  const popups = useMemo(() => ({ slash: new Popup(), mention: new Popup() }), []);
  const callbacks = useRef({ pickBoard, newBoard, onHeadings, titleInput });
  callbacks.current = { pickBoard, newBoard, onHeadings, titleInput };
  const fileInput = useRef<HTMLInputElement>(null);
  const editorRef = useRef<Editor | null>(null);
  // Which board items the slash menu offers is fixed when the editor is created (its extensions are).
  const hasPickBoard = pickBoard !== undefined;
  const hasNewBoard = newBoard !== undefined;
  const ctx: SlashContext = useMemo(
    () => ({
      pickImage: () => fileInput.current?.click(),
      ...(hasPickBoard ? { pickBoard: (e: Editor) => callbacks.current.pickBoard?.(e) } : {}),
      ...(hasNewBoard ? { newBoard: (e: Editor) => callbacks.current.newBoard?.(e) } : {}),
    }),
    [hasPickBoard, hasNewBoard],
  );
  const lastHeadings = useRef<Heading[]>([]);
  const navigate = useNavigate();
  const navigateRef = useRef(navigate);
  navigateRef.current = navigate;
  const openLink = useMemo(
    () => (target: LinkTarget) => {
      if (target.kind === 'route') void navigateRef.current(target.to);
      else openOutside(target);
    },
    [],
  );
  const editor = useEditor(
    {
      extensions: editorExtensions({
        itemId,
        ydoc: session.ydoc,
        slash: popups.slash,
        mention: popups.mention,
        ctx,
        openLink,
        variant,
        ...(callbacks.current.titleInput ? { titleInput: () => callbacks.current.titleInput?.() ?? null } : {}),
      }),
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
    <div className="quire-editor-shell" data-readonly={readOnly || undefined} data-variant={embed ? 'embed' : undefined}>
      {editor && !readOnly && (
        <>
          <SelectionMenu editor={editor} appendTo={portal} />
          <TableMenu editor={editor} appendTo={portal} />
          {!embed && <BlockHandle editor={editor} />}
        </>
      )}
      <EditorContent editor={editor} />
      <SuggestPopup popup={popups.slash} label="Insert a block" portal={portal} />
      <SuggestPopup popup={popups.mention} label="Mention a page" portal={portal} />
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
