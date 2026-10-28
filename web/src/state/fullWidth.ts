// The documents' "Full width" setting chosen in this session (docs/ARCHITECTURE.md §9.6.4 "⋯ … Full width (docs)",
// §7.2 `PATCH /items/{id} {full_width}`; W3-D9). `ItemDetail` has no `full_width` field yet (request W1-2 in
// docs/handoff/W3.md), so every refetch of `GET /items/{id}` would drop a value kept only in the query cache. This store
// keeps it until the page reloads; once the DTO carries the field, the server's value wins (`fullWidthOf` in
// components/item/ItemMenu.tsx).
import { create } from 'zustand';

interface FullWidthState {
  /** By item id. */
  byId: Readonly<Record<string, boolean>>;
}

export const useFullWidthStore = create<FullWidthState>(() => ({ byId: {} }));

export function setFullWidthSetting(itemId: string, value: boolean): void {
  useFullWidthStore.setState((s) => ({ byId: { ...s.byId, [itemId]: value } }));
}

/** The value set in this session, or undefined when none was. */
export function fullWidthSetting(itemId: string): boolean | undefined {
  return useFullWidthStore.getState().byId[itemId];
}

/** Forgets every value (tests). */
export function resetFullWidth(): void {
  useFullWidthStore.setState({ byId: {} });
}
