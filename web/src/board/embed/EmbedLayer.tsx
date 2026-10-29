// The board embeds overlay (docs/ARCHITECTURE.md §18.5): `div.board-embeds`, after the iframe in the frame host's
// fixed layer and placed by the same routine to the same rect, so frame px are overlay-local px. It is
// `overflow: clip` and `pointer-events: none`, hidden and inert whenever the iframe is, and clipped to the canvas minus
// the engine's own interface. Inside, `div.board-embeds-world` carries the camera transform, and each drawn card is
// positioned in world units in it. The controller applies every layout to this DOM directly; React renders only when
// the drawn cards change. A drag of a sidebar document over the editing card (the one part that takes pointer events)
// is taken in the capture phase, before ProseMirror sees it, and placed like a canvas drop.
// The cards (TipTap, the editor) are a chunk of their own, loaded when a board is first shown: the overlay itself is
// mounted on every page with the frame host.
import { useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useState } from 'react';
import { EmbedController, useEmbedStore } from './controller';
import { DRAG_ITEM_TYPE, parseDraggedItem } from './dragItem';
import type { EmbedCard as EmbedCardType } from './EmbedCard';

let Card: typeof EmbedCardType | null = null;
let loading: Promise<void> | null = null;

/** Loads the cards' chunk (once); resolves when cards can render. */
export function loadEmbedCards(): Promise<void> {
  loading ??= import('./EmbedCard').then(
    (m) => {
      Card = m.EmbedCard;
    },
    (e: unknown) => {
      loading = null;
      throw e;
    },
  );
  return loading;
}

export interface EmbedLayerProps {
  /** The iframe is shown (a board route, the board open, no failure). */
  visible: boolean;
  /** The overlay element, for the frame host's placement routine. */
  overlayRef(el: HTMLDivElement | null): void;
}

function carriesItem(e: DragEvent): boolean {
  return [...(e.dataTransfer?.types ?? [])].includes(DRAG_ITEM_TYPE);
}

export function EmbedLayer({ visible, overlayRef }: EmbedLayerProps) {
  const qc = useQueryClient();
  const [controller] = useState(() => new EmbedController(qc));
  const cards = useEmbedStore((s) => s.cards);
  const editing = useEmbedStore((s) => s.editing);
  const readOnly = useEmbedStore((s) => s.board?.readOnly ?? true);
  const [overlay, setOverlay] = useState<HTMLDivElement | null>(null);
  const [CardView, setCardView] = useState<typeof EmbedCardType | null>(() => Card);

  useEffect(() => {
    controller.attach();
    return () => controller.dispose();
  }, [controller]);

  useEffect(() => {
    if (!visible) controller.hidden();
  }, [visible, controller]);

  // The cards' chunk, once a board is shown.
  useEffect(() => {
    if (!visible || CardView) return;
    let alive = true;
    loadEmbedCards().then(
      () => {
        if (alive) setCardView(() => Card);
      },
      (e: unknown) => console.warn('loading the board cards failed', e),
    );
    return () => {
      alive = false;
    };
  }, [visible, CardView]);

  const ref = useCallback(
    (el: HTMLDivElement | null) => {
      controller.setOverlay(el);
      overlayRef(el);
      setOverlay(el);
    },
    [controller, overlayRef],
  );

  // Drops on the editing card: capture phase, so ProseMirror never handles a sidebar row as text.
  useEffect(() => {
    if (!overlay) return;
    const accept = (e: DragEvent) => {
      if (!carriesItem(e)) return;
      e.preventDefault();
      e.stopPropagation();
    };
    const drop = (e: DragEvent) => {
      if (!carriesItem(e)) return;
      e.preventDefault();
      e.stopPropagation();
      const item = parseDraggedItem(e.dataTransfer?.getData(DRAG_ITEM_TYPE));
      if (item) void controller.dropAtViewport(item, e.clientX, e.clientY);
    };
    overlay.addEventListener('dragenter', accept, true);
    overlay.addEventListener('dragover', accept, true);
    overlay.addEventListener('drop', drop, true);
    return () => {
      overlay.removeEventListener('dragenter', accept, true);
      overlay.removeEventListener('dragover', accept, true);
      overlay.removeEventListener('drop', drop, true);
    };
  }, [overlay, controller]);

  return (
    <div ref={ref} className="board-embeds" data-testid="board-embeds" hidden={!visible} inert={!visible}>
      <div ref={controller.setWorld} className="board-embeds-world">
        {CardView &&
          cards.map((c) => (
            <CardView
              key={c.objectId}
              card={c}
              editing={editing?.objectId === c.objectId ? editing : null}
              readOnly={readOnly}
              controller={controller}
            />
          ))}
      </div>
    </div>
  );
}
