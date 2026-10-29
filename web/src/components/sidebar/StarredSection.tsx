// The STARRED section (docs/ARCHITECTURE.md §9.6.2, §9.6.3): shown only when something is starred; rows reorder by
// drag and drop, before/after only (`PUT /items/{id}/star {position}`).
import { combine } from '@atlaskit/pragmatic-drag-and-drop/combine';
import {
  draggable,
  dropTargetForElements,
  monitorForElements,
} from '@atlaskit/pragmatic-drag-and-drop/adapter/element-adapter';
import { useEffect, useRef, useState } from 'react';
import { Link, useLocation } from 'react-router';
import type { ItemSummary } from '../../api/gen/ItemSummary';
import { useStar, type StarVars } from '../../api/queries/items';
import { itemPath } from '../../lib/urls';
import { errorToast } from '../../state/toasts';
import { KindIcon } from '../item/KindIcon';
import { edgeZone } from '../tree/hitZones';
import { listDropPosition } from '../tree/positions';
import { currentTarget } from './CollectionsTree';

export const DRAG_STAR = 'star';

/** The `useStar` variables of dropping star `itemId` before/after star `targetId`; null when nothing would change. */
export function starReorderVars(
  stars: readonly ItemSummary[],
  itemId: string,
  targetId: string,
  edge: 'before' | 'after',
): StarVars | null {
  const position = listDropPosition(stars.map((s) => s.id), itemId, targetId, edge);
  return position ? { itemId, starred: true, position } : null;
}

interface StarIndicator {
  id: string;
  edge: 'before' | 'after';
}

function StarRow({ star, current, indicator }: { star: ItemSummary; current: boolean; indicator: StarIndicator | null }) {
  const el = useRef<HTMLLIElement>(null);
  useEffect(() => {
    const element = el.current;
    if (!element) return;
    return combine(
      draggable({ element, getInitialData: () => ({ wikanvasDrag: DRAG_STAR, itemId: star.id }) }),
      dropTargetForElements({
        element,
        getData: () => ({ wikanvasStar: star.id }),
        canDrop: ({ source }) => source.data.wikanvasDrag === DRAG_STAR,
      }),
    );
  }, [star.id]);
  const cls = ['sidebar-row', 'star-row', current ? 'current' : '', indicator ? `drop-line-${indicator.edge}` : '']
    .filter(Boolean).join(' ');
  return (
    <li ref={el} className={cls}>
      <Link to={itemPath(star)} aria-current={current ? 'page' : undefined} draggable={false}>
        <KindIcon kind={star.kind} />
        <span className="row-text">{star.title || 'Untitled'}</span>
      </Link>
    </li>
  );
}

export function StarredSection({ stars }: { stars: readonly ItemSummary[] }) {
  const location = useLocation();
  const current = currentTarget(location.pathname);
  const star = useStar().mutateAsync;
  const [indicator, setIndicator] = useState<StarIndicator | null>(null);
  const live = useRef({ stars, star });
  live.current = { stars, star };

  useEffect(() => {
    let pending: { itemId: string; targetId: string; edge: 'before' | 'after' } | null = null;
    type Args = Parameters<NonNullable<Parameters<typeof monitorForElements>[0]['onDrag']>>[0];
    const update = ({ source, location: loc }: Args) => {
      const target = loc.current.dropTargets.find((t) => typeof t.data.wikanvasStar === 'string');
      if (!target) {
        pending = null;
        setIndicator(null);
        return;
      }
      const edge = edgeZone(loc.current.input.clientY, target.element.getBoundingClientRect());
      const targetId = String(target.data.wikanvasStar);
      const itemId = String(source.data.itemId);
      pending = itemId === targetId ? null : { itemId, targetId, edge };
      const next = pending ? { id: targetId, edge } : null;
      setIndicator((cur) => (next && cur?.id === next.id && cur.edge === next.edge ? cur : next));
    };
    return monitorForElements({
      canMonitor: ({ source }) => source.data.wikanvasDrag === DRAG_STAR,
      onDrag: update,
      onDropTargetChange: update,
      onDrop: () => {
        setIndicator(null);
        const p = pending;
        pending = null;
        if (!p) return;
        const { stars: list, star: mutate } = live.current;
        const vars = starReorderVars(list, p.itemId, p.targetId, p.edge);
        if (vars) void mutate(vars).catch((e: unknown) => errorToast(e));
      },
    });
  }, []);

  if (stars.length === 0) return null;
  return (
    <section className="sidebar-section" aria-labelledby="sidebar-starred">
      <div className="sidebar-section-label" id="sidebar-starred">Starred</div>
      <ul className="star-list">
        {stars.map((s) => (
          <StarRow
            key={s.id}
            star={s}
            current={current?.kind === 'item' && current.urlId === s.url_id}
            indicator={indicator?.id === s.id ? indicator : null}
          />
        ))}
      </ul>
    </section>
  );
}
