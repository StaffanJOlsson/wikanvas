// A collection's icon (docs/ARCHITECTURE.md §9.6.2): its lucide icon tinted with its colour, or a letter tile.
import type { Collection } from '../../api/gen/Collection';
import { collectionIcon, DEFAULT_COLLECTION_COLOR } from './icons';

export function CollectionIcon({ collection, size = 16 }: { collection: Pick<Collection, 'name' | 'icon' | 'color'>; size?: number }) {
  const Icon = collectionIcon(collection.icon);
  const color = collection.color || DEFAULT_COLLECTION_COLOR;
  if (Icon) return <Icon className="collection-icon" size={size} color={color} aria-hidden />;
  const letter = [...collection.name.trim()][0]?.toUpperCase() ?? '?';
  return (
    <span className="letter-tile" aria-hidden style={{ background: color, width: size, height: size, fontSize: size * 0.62 }}>
      {letter}
    </span>
  );
}
