import { useEffect } from "react";
import {
  COCKPIT_HOTKEYS,
  isTextEditingTarget,
  type CockpitHotkeyAction,
  type CockpitHotkeyHandlers,
} from "../state/cockpit-operation-semantics";

export function useCockpitHotkeys(handlers: CockpitHotkeyHandlers): void {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.altKey || isTextEditingTarget(event.target)) {
        return;
      }
      const action = (Object.keys(COCKPIT_HOTKEYS) as CockpitHotkeyAction[]).find(
        (candidate) => {
          const binding = COCKPIT_HOTKEYS[candidate];
          return (
            event.code === binding.code &&
            (event.ctrlKey || event.metaKey) === binding.ctrlOrMeta &&
            event.shiftKey === binding.shift
          );
        },
      );
      if (!action) {
        return;
      }
      event.preventDefault();
      handlers[action]();
    };

    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [handlers]);
}
