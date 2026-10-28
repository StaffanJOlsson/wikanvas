// The editor's node views (docs/ARCHITECTURE.md §6.1, §9.6.5): mentions with live titles and dead-target states,
// the board embed card (the page-0 SVG preview, never the engine), images (uploaded, with width handles, alt and
// caption; external ones as a placeholder that is never loaded), callouts, and code blocks with a language select.
import { NodeViewContent, NodeViewWrapper, type NodeViewProps } from '@tiptap/react';
import { AlertTriangle, ExternalLink, Info, Lightbulb, OctagonAlert, PenTool } from 'lucide-react';
import type { ReactNode } from 'react';
import { Link } from 'react-router';
import { isApiError } from '../api/client';
import { useItem } from '../api/queries/items';
import { attachmentUrl, boardPreviewUrl, itemPath } from '../lib/urls';
import { isAllowedImageSrc } from './links';
import { CALLOUT_VARIANTS, IMAGE_WIDTHS, type CalloutVariant } from './schema';

/** A mention: the item's live title; a trashed target struck through with "In Trash"; a missing one "Deleted item". */
export function MentionChip({ itemId }: { itemId: string | null }) {
  const q = useItem(itemId ?? null);
  if (!itemId || isApiError(q.error, 'not_found')) {
    return (
      <span className="mention mention-missing" aria-disabled="true">
        @Deleted item
      </span>
    );
  }
  if (q.isPending) return <span className="mention mention-loading">@…</span>;
  if (q.isError) return <span className="mention mention-missing">@Unavailable</span>;
  const d = q.data;
  const title = d.item.title || 'Untitled';
  if (d.trashed_at !== null) {
    return (
      <Link to={itemPath(d.item)} className="mention mention-trashed" title="In Trash">
        <s>@{title}</s> <span className="mention-note">In Trash</span>
      </Link>
    );
  }
  return (
    <Link to={itemPath(d.item)} className="mention">
      @{title}
    </Link>
  );
}

export function MentionView({ node }: NodeViewProps) {
  return (
    <NodeViewWrapper as="span" className="mention-view">
      <MentionChip itemId={(node.attrs.itemId as string | null) ?? null} />
    </NodeViewWrapper>
  );
}

/** A board card: the page-0 preview `<img>` and the title, linking to the board. Never loads `/inkboard/`. */
export function BoardEmbedCard({ itemId }: { itemId: string | null }) {
  const q = useItem(itemId ?? null);
  if (!itemId || isApiError(q.error, 'not_found')) {
    return <div className="board-embed board-embed-missing">Deleted board</div>;
  }
  if (q.isPending) return <div className="board-embed board-embed-loading">Loading board…</div>;
  if (q.isError) return <div className="board-embed board-embed-missing">Board unavailable</div>;
  const d = q.data;
  const board = d.board;
  const title = d.item.title || 'Untitled board';
  const preview =
    board && board.pages.length > 0 && board.preview_rev !== null ? boardPreviewUrl(d.item.id, board.rev, 0) : null;
  return (
    <Link to={itemPath(d.item)} className="board-embed" data-trashed={d.trashed_at !== null || undefined}>
      {preview ? (
        <img className="board-embed-preview" src={preview} alt={board?.text_excerpt || title} loading="lazy" />
      ) : (
        <span className="board-embed-empty" aria-hidden>
          <PenTool size={24} />
        </span>
      )}
      <span className="board-embed-title">{title}</span>
    </Link>
  );
}

export function BoardEmbedView({ node, selected }: NodeViewProps) {
  return (
    <NodeViewWrapper className="board-embed-view" data-selected={selected || undefined} data-drag-handle>
      <BoardEmbedCard itemId={(node.attrs.itemId as string | null) ?? null} />
    </NodeViewWrapper>
  );
}

/** An external image: a card that says so and links to it; the image itself is never requested. */
export function ExternalImageCard({ src, alt }: { src: string | null; alt: string | null }) {
  const safe = src && isAllowedImageSrc(src) ? src : null;
  return (
    <div className="external-image" role="img" aria-label={alt ?? 'External image'}>
      <span>External image, not loaded</span>
      {safe && (
        <a href={safe} target="_blank" rel="noopener noreferrer nofollow">
          Open link <ExternalLink size={12} aria-hidden />
        </a>
      )}
    </div>
  );
}

export function ImageView({ node, updateAttributes, selected, editor }: NodeViewProps) {
  const { attachmentId, src, width, alt, caption } = node.attrs as {
    attachmentId: string | null;
    src: string | null;
    width: number | null;
    alt: string | null;
    caption: string | null;
  };
  const editable = editor.isEditable;
  return (
    <NodeViewWrapper as="figure" className="image-view" data-selected={selected || undefined} data-width={width ?? undefined}>
      {attachmentId ? (
        <img src={attachmentUrl(attachmentId)} alt={alt ?? ''} style={width ? { width: `${width}%` } : undefined} draggable={false} />
      ) : (
        <ExternalImageCard src={src} alt={alt} />
      )}
      {editable && selected && attachmentId && (
        <div className="image-tools" contentEditable={false}>
          {IMAGE_WIDTHS.map((w) => (
            <button key={w} type="button" aria-pressed={width === w} onClick={() => updateAttributes({ width: w })}>
              {w}%
            </button>
          ))}
          <button
            type="button"
            onClick={() => {
              const next = window.prompt('Alt text', alt ?? '');
              if (next !== null) updateAttributes({ alt: next.trim() || null });
            }}
          >
            Alt text
          </button>
        </div>
      )}
      {(caption || (editable && selected)) && (
        <figcaption contentEditable={false}>
          {editable ? (
            <input
              className="image-caption"
              aria-label="Caption"
              placeholder="Add a caption"
              value={caption ?? ''}
              onChange={(e) => updateAttributes({ caption: e.target.value || null })}
            />
          ) : (
            caption
          )}
        </figcaption>
      )}
    </NodeViewWrapper>
  );
}

const CALLOUT_ICONS: Record<CalloutVariant, ReactNode> = {
  info: <Info size={16} aria-hidden />,
  tip: <Lightbulb size={16} aria-hidden />,
  warning: <AlertTriangle size={16} aria-hidden />,
  danger: <OctagonAlert size={16} aria-hidden />,
};

export function CalloutView({ node, updateAttributes, editor }: NodeViewProps) {
  const variant = (CALLOUT_VARIANTS as readonly string[]).includes(node.attrs.variant as string)
    ? (node.attrs.variant as CalloutVariant)
    : 'info';
  return (
    <NodeViewWrapper className="callout" data-variant={variant}>
      <span className="callout-icon" contentEditable={false}>
        {editor.isEditable ? (
          <select aria-label="Callout type" value={variant} onChange={(e) => updateAttributes({ variant: e.target.value })}>
            {CALLOUT_VARIANTS.map((v) => (
              <option key={v} value={v}>
                {v}
              </option>
            ))}
          </select>
        ) : null}
        {CALLOUT_ICONS[variant]}
      </span>
      <NodeViewContent className="callout-content" />
    </NodeViewWrapper>
  );
}

/** Languages offered in the code block select (lowlight's common set, by its names). */
export const CODE_LANGUAGES = [
  'bash', 'c', 'cpp', 'csharp', 'css', 'diff', 'go', 'graphql', 'ini', 'java', 'javascript', 'json', 'kotlin',
  'less', 'lua', 'makefile', 'markdown', 'objectivec', 'perl', 'php', 'plaintext', 'python', 'r', 'ruby', 'rust',
  'scss', 'shell', 'sql', 'swift', 'typescript', 'vbnet', 'wasm', 'xml', 'yaml',
];

export function CodeBlockView({ node, updateAttributes, editor }: NodeViewProps) {
  const language = (node.attrs.language as string | null) ?? '';
  return (
    <NodeViewWrapper className="code-block">
      {editor.isEditable && (
        <select
          className="code-language"
          contentEditable={false}
          aria-label="Language"
          value={language}
          onChange={(e) => updateAttributes({ language: e.target.value || null })}
        >
          <option value="">Plain text</option>
          {(CODE_LANGUAGES.includes(language) || language === '' ? CODE_LANGUAGES : [language, ...CODE_LANGUAGES]).map((l) => (
            <option key={l} value={l}>
              {l}
            </option>
          ))}
        </select>
      )}
      <pre>
        <NodeViewContent<'code'> as="code" className={language ? `language-${language}` : undefined} />
      </pre>
    </NodeViewWrapper>
  );
}
