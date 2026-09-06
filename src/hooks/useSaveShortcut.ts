/**
 * useSaveShortcut — binds `Ctrl+S` / `Cmd+S` on `window` to a save callback
 * (PRD.md RF-083 "botão + `Ctrl+S`"; RNF-013 keyboard navigation). The combo
 * is swallowed with `preventDefault()` so the browser's native "Save Page"
 * dialog never flashes — but only once the shortcut is actually armed:
 * while `disabled` the keydown is ignored entirely (e.g. nothing to save,
 * or a save is already in flight), matching `SettingsFooter`'s Save button
 * being disabled in the same conditions.
 */
import { useEffect } from "react";

export type UseSaveShortcutOptions = {
  disabled?: boolean;
};

export function useSaveShortcut(onSave: () => void, options: UseSaveShortcutOptions = {}): void {
  const { disabled = false } = options;

  useEffect(() => {
    if (disabled) return;

    function handleKeyDown(event: KeyboardEvent): void {
      const isSaveCombo = (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s";
      if (!isSaveCombo) return;

      event.preventDefault();
      onSave();
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onSave, disabled]);
}
