import { useEffect, useRef } from "react";

export interface Shortcut {
  /** `KeyboardEvent.key`, lower-case (`"1"`, `","`, `"r"`). */
  key: string;
  shift?: boolean;
  enabled?: boolean;
  handler: () => void;
}

const matches = (e: KeyboardEvent, s: Shortcut) =>
  (e.metaKey || e.ctrlKey) &&
  !e.altKey &&
  e.shiftKey === !!s.shift &&
  e.key.toLowerCase() === s.key;

/** ⌘ (or Ctrl) shortcuts handled in the webview; native menus come later. */
export function useShortcuts(shortcuts: Shortcut[]) {
  const ref = useRef(shortcuts);
  useEffect(() => {
    ref.current = shortcuts;
  });
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat) return;
      const hit = ref.current.find((s) => s.enabled !== false && matches(e, s));
      if (!hit) return;
      e.preventDefault();
      hit.handler();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
