// The sidebar's resize handle (docs/ARCHITECTURE.md §9.3, §9.6.1): drag between 216 and 400 px, double-click resets
// to 260; ←/→ resize by 16 px for keyboard users.
import { useRef } from 'react';
import { resetSidebarWidth, setSidebarWidth, SIDEBAR_MAX, SIDEBAR_MIN, useUi } from '../../state/ui';

export function ResizeHandle() {
  const width = useUi((s) => s.sidebarWidth);
  const drag = useRef<{ x: number; w: number } | null>(null);
  return (
    <div
      className="resize-handle"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize sidebar"
      aria-valuemin={SIDEBAR_MIN}
      aria-valuemax={SIDEBAR_MAX}
      aria-valuenow={width}
      tabIndex={0}
      onPointerDown={(e) => {
        drag.current = { x: e.clientX, w: width };
        e.currentTarget.setPointerCapture?.(e.pointerId);
        document.body.classList.add('resizing');
      }}
      onPointerMove={(e) => {
        if (drag.current) setSidebarWidth(drag.current.w + e.clientX - drag.current.x);
      }}
      onPointerUp={(e) => {
        drag.current = null;
        e.currentTarget.releasePointerCapture?.(e.pointerId);
        document.body.classList.remove('resizing');
      }}
      onDoubleClick={() => resetSidebarWidth()}
      onKeyDown={(e) => {
        if (e.key === 'ArrowLeft') setSidebarWidth(width - 16);
        else if (e.key === 'ArrowRight') setSidebarWidth(width + 16);
        else return;
        e.preventDefault();
      }}
    />
  );
}
