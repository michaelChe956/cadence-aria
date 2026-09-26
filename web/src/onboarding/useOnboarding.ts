import { useEffect, useMemo, useState } from "react";
import { getEnabledOnboardingSteps, type OnboardingStep } from "./steps";

/** 设备级已读标记键（与 whats-new 的 aria-whats-new-seen 并列，互不影响）。 */
export const ONBOARDING_SEEN_KEY = "aria-onboarding-seen";

const STORAGE_UNAVAILABLE = Symbol("onboarding-storage-unavailable");

function readSeen(): boolean | typeof STORAGE_UNAVAILABLE {
  try {
    return window.localStorage.getItem(ONBOARDING_SEEN_KEY) !== null;
  } catch {
    return STORAGE_UNAVAILABLE;
  }
}

function writeSeen(): void {
  try {
    window.localStorage.setItem(ONBOARDING_SEEN_KEY, "1");
  } catch {
    /* localStorage 不可用：静默降级，不阻断工作台。 */
  }
}

export interface UseOnboardingResult {
  open: boolean;
  index: number;
  steps: OnboardingStep[];
  total: number;
  current: OnboardingStep | null;
  /** 下一步（在启用步骤边界内收敛）。 */
  next: () => void;
  /** 上一步（在启用步骤边界内收敛）。 */
  prev: () => void;
  /** 跳过整个引导：记录已读并关闭。 */
  skip: () => void;
  /** 完成 / 关闭：记录已读并关闭。 */
  close: () => void;
  /** 帮助入口重开：只重置内存 open/index，不清除已读记录。 */
  reopen: () => void;
}

/**
 * onboarding 生命周期与设备级已读判定（沿 useWhatsNew 的静默降级先例）。
 *
 * - 首次进入（无已读记录）且有启用步骤时自动打开；
 * - 只有 skip / 完成 / 关闭才写入已读，卸载不误记；
 * - localStorage 异常时静默降级（不自动展示、不抛错），帮助入口仍可手动重开。
 */
export function useOnboarding(): UseOnboardingResult {
  const steps = useMemo(() => getEnabledOnboardingSteps(), []);
  const [open, setOpen] = useState(false);
  const [index, setIndex] = useState(0);

  useEffect(() => {
    const seen = readSeen();
    if (seen === STORAGE_UNAVAILABLE) {
      setOpen(false);
      return;
    }
    setOpen(!seen && steps.length > 0);
  }, [steps.length]);

  const lastIndex = Math.max(steps.length - 1, 0);

  return {
    open,
    index,
    steps,
    total: steps.length,
    current: steps[index] ?? null,
    next: () => setIndex((current) => Math.min(current + 1, lastIndex)),
    prev: () => setIndex((current) => Math.max(current - 1, 0)),
    skip: () => {
      writeSeen();
      setOpen(false);
    },
    close: () => {
      writeSeen();
      setOpen(false);
    },
    reopen: () => {
      setIndex(0);
      setOpen(true);
    },
  };
}
