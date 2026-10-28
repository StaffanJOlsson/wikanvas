// The star toggle of the item header and of the board page's top bar (docs/ARCHITECTURE.md §9.3, §9.6.4).
import { Star } from 'lucide-react';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import { useStar } from '../../api/queries/items';
import { errorToast } from '../../state/toasts';

export interface StarToggleProps {
  item: ItemDetail;
}

export function StarToggle({ item }: StarToggleProps) {
  const star = useStar();
  const starred = item.starred;
  return (
    <button
      type="button"
      className="icon-button star-toggle"
      aria-pressed={starred}
      aria-label={starred ? 'Unstar' : 'Star'}
      title={starred ? 'Unstar' : 'Star'}
      onClick={() => star.mutate({ itemId: item.item.id, starred: !starred, item: item.item }, { onError: (e) => errorToast(e) })}
    >
      <Star size={16} aria-hidden fill={starred ? 'currentColor' : 'none'} />
    </button>
  );
}
