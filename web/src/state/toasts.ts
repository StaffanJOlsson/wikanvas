// Toasts (docs/ARCHITECTURE.md §9.2, §9.6.3): short messages at the bottom of the screen, optionally with one action
// ("Undo"). Rendered by components/toasts/Toasts.
import { create } from 'zustand';
import { isApiError } from '../api/client';

export type ToastKind = 'info' | 'success' | 'error';

export interface ToastAction {
  label: string;
  run(): void | Promise<void>;
}

export interface Toast {
  id: number;
  message: string;
  kind: ToastKind;
  action?: ToastAction;
  /** Milliseconds until it disappears. */
  duration: number;
}

export interface ToastInput {
  message: string;
  kind?: ToastKind;
  action?: ToastAction;
  duration?: number;
}

export const TOAST_DURATION = 5000;
/** The drop toast "Moved to … · Undo" stays 8 s (§9.6.3). */
export const UNDO_DURATION = 8000;
const MAX_TOASTS = 4;

interface ToastState {
  toasts: Toast[];
}

export const useToasts = create<ToastState>(() => ({ toasts: [] }));

let nextId = 1;
const timers = new Map<number, ReturnType<typeof setTimeout>>();

export function dismissToast(id: number): void {
  const t = timers.get(id);
  if (t !== undefined) clearTimeout(t);
  timers.delete(id);
  useToasts.setState((s) => ({ toasts: s.toasts.filter((x) => x.id !== id) }));
}

/** Shows a toast; returns its id. */
export function toast(input: ToastInput): number {
  const id = nextId++;
  const t: Toast = {
    id,
    message: input.message,
    kind: input.kind ?? 'info',
    action: input.action,
    duration: input.duration ?? (input.action ? UNDO_DURATION : TOAST_DURATION),
  };
  useToasts.setState((s) => {
    const toasts = [...s.toasts, t];
    for (const old of toasts.slice(0, Math.max(0, toasts.length - MAX_TOASTS))) {
      const timer = timers.get(old.id);
      if (timer !== undefined) clearTimeout(timer);
      timers.delete(old.id);
    }
    return { toasts: toasts.slice(-MAX_TOASTS) };
  });
  timers.set(id, setTimeout(() => dismissToast(id), t.duration));
  return id;
}

/** The English message of an error for a toast. */
export function errorMessage(e: unknown, fallback = 'Something went wrong'): string {
  if (isApiError(e)) {
    if (e.code === 'network') return 'You are offline, or the server cannot be reached';
    if (e.code === 'cycle') return "An item can't be moved into itself or one of its nested pages";
    return e.message || fallback;
  }
  return e instanceof Error && e.message ? e.message : fallback;
}

export function errorToast(e: unknown, fallback?: string): number {
  return toast({ message: errorMessage(e, fallback), kind: 'error' });
}

/** Removes every toast (tests, sign-out). */
export function clearToasts(): void {
  for (const t of timers.values()) clearTimeout(t);
  timers.clear();
  useToasts.setState({ toasts: [] });
}
