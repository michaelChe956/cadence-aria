import { useCallback, useEffect, useRef, useState } from "react";

interface UseWorkspaceWsReconnectOptions {
  enabled: boolean;
  onReconnect: () => void;
  closeCode?: number;
}

const INITIAL_DELAY_MS = 1000;
const MAX_DELAY_MS = 16000;
const JITTER_PCT = 0.2;

function nextDelay(previousDelay: number): number {
  const baseDelay = Math.min(previousDelay * 2, MAX_DELAY_MS);
  const jitter = baseDelay * JITTER_PCT * (Math.random() * 2 - 1);
  return Math.max(INITIAL_DELAY_MS, Math.round(baseDelay + jitter));
}

export function useWorkspaceWsReconnect({
  enabled,
  onReconnect,
  closeCode,
}: UseWorkspaceWsReconnectOptions) {
  const [attemptCount, setAttemptCount] = useState(0);
  const [isReconnecting, setIsReconnecting] = useState(false);
  const delayRef = useRef(INITIAL_DELAY_MS);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const onReconnectRef = useRef(onReconnect);
  const scheduleReconnectRef = useRef<() => void>(() => undefined);
  onReconnectRef.current = onReconnect;

  const clearReconnectTimeout = useCallback(() => {
    if (timeoutRef.current) {
      clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
  }, []);

  const shouldReconnect = useCallback(() => {
    return enabled && closeCode !== 1000 && !document.hidden;
  }, [closeCode, enabled]);

  const scheduleReconnect = useCallback(() => {
    clearReconnectTimeout();
    if (!shouldReconnect()) {
      return;
    }

    setIsReconnecting(true);
    const delay = delayRef.current;
    timeoutRef.current = setTimeout(() => {
      timeoutRef.current = null;
      setAttemptCount((count) => count + 1);
      onReconnectRef.current();
      delayRef.current = nextDelay(delay);
      scheduleReconnectRef.current();
    }, delay);
  }, [clearReconnectTimeout, shouldReconnect]);

  useEffect(() => {
    scheduleReconnectRef.current = scheduleReconnect;
  }, [scheduleReconnect]);

  useEffect(() => {
    if (closeCode === 1000) {
      clearReconnectTimeout();
      setIsReconnecting(false);
      return;
    }
    if (!enabled) {
      clearReconnectTimeout();
      return;
    }

    scheduleReconnect();
    return () => clearReconnectTimeout();
  }, [clearReconnectTimeout, closeCode, enabled, scheduleReconnect]);

  useEffect(() => {
    function handleVisibilityChange() {
      if (document.hidden) {
        clearReconnectTimeout();
        setIsReconnecting(false);
        return;
      }
      delayRef.current = INITIAL_DELAY_MS;
      scheduleReconnectRef.current();
    }

    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [clearReconnectTimeout]);

  const reset = useCallback(() => {
    clearReconnectTimeout();
    delayRef.current = INITIAL_DELAY_MS;
    setAttemptCount(0);
    setIsReconnecting(false);
  }, [clearReconnectTimeout]);

  const retryNow = useCallback(() => {
    clearReconnectTimeout();
    if (!shouldReconnect()) {
      return;
    }

    setIsReconnecting(true);
    setAttemptCount((count) => count + 1);
    onReconnectRef.current();
    delayRef.current = nextDelay(INITIAL_DELAY_MS);
    scheduleReconnectRef.current();
  }, [clearReconnectTimeout, shouldReconnect]);

  return {
    isReconnecting,
    attemptCount,
    retryNow,
    reset,
  };
}
