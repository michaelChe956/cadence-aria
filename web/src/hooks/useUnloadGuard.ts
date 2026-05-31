import { useBlocker } from "@tanstack/react-router";
import { useEffect, useRef } from "react";

interface UseUnloadGuardOptions {
  enabled: boolean;
  message: string;
}

export function useUnloadGuard({ enabled, message }: UseUnloadGuardOptions) {
  const messageRef = useRef(message);
  messageRef.current = message;

  useEffect(() => {
    if (!enabled) return;

    function handleBeforeUnload(event: BeforeUnloadEvent) {
      event.preventDefault();
      event.returnValue = messageRef.current;
      return messageRef.current;
    }

    window.addEventListener("beforeunload", handleBeforeUnload);
    return () => {
      window.removeEventListener("beforeunload", handleBeforeUnload);
    };
  }, [enabled]);

  useBlocker({
    condition: enabled,
    blockerFn: () => !window.confirm(messageRef.current),
  });
}
