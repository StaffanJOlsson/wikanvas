// The board page, lazily loaded at /board/:slugId (docs/ARCHITECTURE.md §9.1, §9.3, §9.5).
// PLACEHOLDER (W0b): the top bar with the editable title crumb and the star, and page 0's SVG preview; W6 replaces it
// with BoardSurface over the frame host, BoardLoading, the save loop and BoardPreview.
import { useParams } from 'react-router';
import { isApiError } from '../api/client';
import { useItem } from '../api/queries/items';
import { useBoardFrame } from '../board/BoardFrameHost';
import { ItemMenu } from '../components/item/ItemMenu';
import { StarToggle } from '../components/item/StarToggle';
import { TopBar } from '../components/topbar/TopBar';
import { boardPreviewUrl, itemPath, urlIdFromSlugId } from '../lib/urls';
import { useCanonicalPath } from '../lib/useCanonicalPath';
import NotFound from './NotFound';
import { PlaceholderPage } from './PlaceholderPage';

export default function BoardPage() {
  const { slugId } = useParams();
  const ref = urlIdFromSlugId(slugId);
  const item = useItem(ref);
  const frame = useBoardFrame();
  useCanonicalPath(item.data ? itemPath(item.data.item) : null);

  if (ref === null || isApiError(item.error, 'not_found')) return <NotFound />;
  if (item.isPending) return null;
  if (item.isError) return <PlaceholderPage title="Board">{item.error.message}</PlaceholderPage>;
  const d = item.data;
  const board = d.board;
  return (
    <article className="board-page">
      <TopBar
        item={d}
        titleCrumb="editable"
        actions={
          <>
            <StarToggle item={d} />
            <ItemMenu item={d} extraExports={[]} />
          </>
        }
      />
      {board && board.pages.length > 0 && board.preview_rev !== null ? (
        <img
          className="board-preview"
          src={boardPreviewUrl(d.item.id, board.rev, 0)}
          alt={board.text_excerpt || d.item.title || 'Board preview'}
        />
      ) : (
        <PlaceholderPage>
          {frame?.failure?.message ?? 'The whiteboard editor comes with W6.'}
        </PlaceholderPage>
      )}
    </article>
  );
}
