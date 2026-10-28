// "Collection / Parent / …" for list rows, search results and the quick switcher (docs/ARCHITECTURE.md §9.6.6).
import type { Crumb } from '../../api/gen/Crumb';
import { useSidebar } from '../../api/queries/tree';

export function pathText(collectionName: string | undefined, path: readonly Crumb[]): string {
  return [collectionName, ...path.map((c) => c.title || 'Untitled')].filter(Boolean).join(' / ');
}

export function useCollectionName(collectionId: string): string | undefined {
  return useSidebar().data?.collections.find((c) => c.id === collectionId)?.name;
}

export function ItemPath({ collectionId, path }: { collectionId: string; path: readonly Crumb[] }) {
  const name = useCollectionName(collectionId);
  const text = pathText(name, path);
  if (!text) return null;
  return <span className="item-path">{text}</span>;
}
