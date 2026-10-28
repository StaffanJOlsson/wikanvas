// A menu button (WAI-ARIA menu pattern): the button toggles a popover with menu items, rendered in a portal so the
// sidebar's overflow never clips it. Keyboard: ↑/↓, Home/End, Enter/Space, Esc (back to the button), Tab closes, and
// each item's accelerator letter (the "+" popover's D and B, §9.6.2).
import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';

export interface MenuItemDef {
  label: string;
  onSelect(): void;
  /** A single letter that selects the item while the menu is open; also shown as a hint. */
  accel?: string;
  danger?: boolean;
  /** Renders a radio item (`menuitemradio`, `aria-checked`), or a checkbox item with `checkbox`. */
  checked?: boolean;
  checkbox?: boolean;
  disabled?: boolean;
  icon?: ReactNode;
}

export type MenuEntry = MenuItemDef | { separator: true } | { heading: string };

export interface MenuProps {
  /** The accessible name of the button and the menu. */
  label: string;
  entries: MenuEntry[];
  /** The button's content (an icon). */
  children: ReactNode;
  buttonClassName?: string;
  /** The menu's edge that lines up with the button. */
  align?: 'start' | 'end';
  title?: string;
  onOpenChange?(open: boolean): void;
  /** Opens without a click (keyboard shortcuts on rows); the menu calls `onOpenChange(false)` when it closes. */
  open?: boolean;
}

function isItem(e: MenuEntry): e is MenuItemDef {
  return 'label' in e;
}

interface Pos {
  top: number;
  left?: number;
  right?: number;
}

export function Menu({ label, entries, children, buttonClassName, align = 'start', title, onOpenChange, open: forced }: MenuProps) {
  const [openState, setOpenState] = useState(false);
  const open = forced ?? openState;
  const [pos, setPos] = useState<Pos | null>(null);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const menuId = useId();

  const setOpen = useCallback((o: boolean) => {
    setOpenState(o);
    onOpenChange?.(o);
  }, [onOpenChange]);

  const close = useCallback((focusButton: boolean) => {
    setOpen(false);
    if (focusButton) button.current?.focus();
  }, [setOpen]);

  useLayoutEffect(() => {
    if (!open || !button.current) return;
    const r = button.current.getBoundingClientRect();
    setPos(align === 'end'
      ? { top: r.bottom + 4, right: Math.max(4, window.innerWidth - r.right) }
      : { top: r.bottom + 4, left: Math.max(4, r.left) });
  }, [open, align]);

  useEffect(() => {
    if (!open) return;
    const first = menu.current?.querySelector<HTMLElement>('[role^="menuitem"]:not([disabled])');
    first?.focus();
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!menu.current?.contains(t) && !button.current?.contains(t)) setOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open, setOpen, pos]);

  const select = (item: MenuItemDef) => {
    if (item.disabled) return;
    close(false);
    item.onSelect();
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const items = [...(menu.current?.querySelectorAll<HTMLElement>('[role^="menuitem"]:not([disabled])') ?? [])];
    const i = items.indexOf(document.activeElement as HTMLElement);
    const focus = (n: number) => items[(n + items.length) % items.length]?.focus();
    e.stopPropagation();
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        focus(i + 1);
        return;
      case 'ArrowUp':
        e.preventDefault();
        focus(i - 1);
        return;
      case 'Home':
        e.preventDefault();
        focus(0);
        return;
      case 'End':
        e.preventDefault();
        focus(items.length - 1);
        return;
      case 'Escape':
        e.preventDefault();
        close(true);
        return;
      case 'Tab':
        close(false);
        return;
    }
    if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
      const hit = entries.find((x): x is MenuItemDef => isItem(x) && !!x.accel && x.accel.toLowerCase() === e.key.toLowerCase());
      if (hit) {
        e.preventDefault();
        select(hit);
      }
    }
  };

  return (
    <>
      <button
        ref={button}
        type="button"
        className={buttonClassName ?? 'icon-button'}
        aria-label={label}
        title={title ?? label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        onClick={(e) => {
          e.stopPropagation();
          setOpen(!open);
        }}
        onKeyDown={(e) => {
          if (e.key === 'ArrowDown' && !open) {
            e.preventDefault();
            e.stopPropagation();
            setOpen(true);
          }
        }}
      >
        {children}
      </button>
      {open && createPortal(
        <div
          ref={menu}
          role="menu"
          id={menuId}
          aria-label={label}
          className="popover menu-popover"
          style={pos ? { top: pos.top, left: pos.left, right: pos.right } : undefined}
          onKeyDown={onKeyDown}
          onClick={(e) => e.stopPropagation()}
        >
          {entries.map((entry, i) => {
            if ('separator' in entry) return <div key={`s${i}`} role="separator" />;
            if ('heading' in entry) return <div key={`h${i}`} className="menu-heading" role="presentation">{entry.heading}</div>;
            return (
              <button
                key={`${entry.label}${i}`}
                type="button"
                role={entry.checked === undefined ? 'menuitem' : entry.checkbox ? 'menuitemcheckbox' : 'menuitemradio'}
                aria-checked={entry.checked === undefined ? undefined : entry.checked}
                className={entry.danger ? 'danger' : undefined}
                disabled={entry.disabled}
                tabIndex={-1}
                onClick={() => select(entry)}
              >
                {entry.icon}
                <span className="menu-label">{entry.label}</span>
                {entry.accel && <kbd className="menu-accel" aria-hidden>{entry.accel}</kbd>}
              </button>
            );
          })}
        </div>,
        document.body,
      )}
    </>
  );
}
