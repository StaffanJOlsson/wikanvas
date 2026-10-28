// UI state (docs/ARCHITECTURE.md §9.2): sidebar width and collapsed state, the drawer below 1024 px, and the theme.
// Width and collapsed state persist in localStorage['quire.ui']; the theme in localStorage['quire.theme'], which
// public/theme-init.js reads before the first paint. The server copy (users.theme) wins once /auth/me has loaded.
import { create } from 'zustand';
import type { Theme } from '../api/gen/Theme';
import { readJson, readStorage, writeStorage } from './storage';

export const UI_KEY = 'quire.ui';
export const THEME_KEY = 'quire.theme';

export const SIDEBAR_DEFAULT = 260;
export const SIDEBAR_MIN = 216;
export const SIDEBAR_MAX = 400;
/** Below this width the sidebar is a drawer (§9.6.1). */
export const DRAWER_QUERY = '(max-width: 1023px)';

export const THEMES: readonly Theme[] = ['light', 'dark', 'system'];

interface UiState {
  sidebarWidth: number;
  sidebarCollapsed: boolean;
  /** The drawer (narrow screens) is open. Never persisted. */
  drawerOpen: boolean;
  theme: Theme;
}

export function clampWidth(w: number): number {
  if (!Number.isFinite(w)) return SIDEBAR_DEFAULT;
  return Math.round(Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, w)));
}

function isTheme(t: unknown): t is Theme {
  return t === 'light' || t === 'dark' || t === 'system';
}

function load(): UiState {
  const saved = readJson(UI_KEY) as Partial<{ sidebarWidth: unknown; sidebarCollapsed: unknown }> | null;
  const stored = readStorage(THEME_KEY);
  return {
    sidebarWidth: typeof saved?.sidebarWidth === 'number' ? clampWidth(saved.sidebarWidth) : SIDEBAR_DEFAULT,
    sidebarCollapsed: saved?.sidebarCollapsed === true,
    drawerOpen: false,
    theme: isTheme(stored) ? stored : 'system',
  };
}

export const useUi = create<UiState>(load);

function persist(): void {
  const { sidebarWidth, sidebarCollapsed } = useUi.getState();
  writeStorage(UI_KEY, JSON.stringify({ sidebarWidth, sidebarCollapsed }));
}

export function setSidebarWidth(w: number): void {
  useUi.setState({ sidebarWidth: clampWidth(w) });
  persist();
}

export function resetSidebarWidth(): void {
  setSidebarWidth(SIDEBAR_DEFAULT);
}

export function setSidebarCollapsed(collapsed: boolean): void {
  useUi.setState({ sidebarCollapsed: collapsed });
  persist();
}

export function setDrawerOpen(open: boolean): void {
  useUi.setState({ drawerOpen: open });
}

/** Whether the viewport is narrow enough for the drawer. */
export function isNarrow(): boolean {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    && window.matchMedia(DRAWER_QUERY).matches;
}

/** Mod+. : toggles the drawer on narrow screens, the collapsed sidebar otherwise. */
export function toggleSidebar(narrow: boolean = isNarrow()): void {
  if (narrow) setDrawerOpen(!useUi.getState().drawerOpen);
  else setSidebarCollapsed(!useUi.getState().sidebarCollapsed);
}

/** Sets `<html data-theme>`; tokens.css resolves `system` with prefers-color-scheme. */
export function applyTheme(theme: Theme): void {
  if (typeof document !== 'undefined') document.documentElement.setAttribute('data-theme', theme);
}

/** Applies a theme and remembers it locally for theme-init.js (the server copy is saved by the caller). */
export function setTheme(theme: Theme): void {
  applyTheme(theme);
  writeStorage(THEME_KEY, theme);
  useUi.setState({ theme });
}

/** Reloads the persisted state (tests, and a storage event from another tab). */
export function reloadUi(): void {
  useUi.setState(load());
}
