// The kind icon of an item row: a page for documents, a whiteboard for boards.
import { FileText, Presentation } from 'lucide-react';
import type { ItemKind } from '../../api/gen/ItemKind';

export function KindIcon({ kind, size = 16 }: { kind: ItemKind; size?: number }) {
  const Icon = kind === 'board' ? Presentation : FileText;
  return <Icon className="kind-icon" size={size} aria-hidden />;
}

export function kindLabel(kind: ItemKind): string {
  return kind === 'board' ? 'Board' : 'Document';
}
