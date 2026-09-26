import { useEffect, useState } from "react";
import { ArrowLeft, ArrowRight, Check, X } from "lucide-react";
import { onboardingProgress, type OnboardingStep } from "./steps";

const HIGHLIGHT_PADDING = 4;

/**
 * 无第三方依赖的引导浮层：说明卡 + 位置/总数 + 上一步/下一步/跳过/完成 +
 * 示意资源降级 + 缺失锚点提示 + 视觉高亮。
 *
 * 高亮层 `pointer-events-none`，不捕获业务点击；说明卡提供非阻塞控制。
 * 步骤切换 / resize / scroll 时重新测量，卸载时清理监听与高亮。
 */
export function OnboardingWizard({
  steps,
  index,
  onPrev,
  onNext,
  onSkip,
  onClose,
}: {
  steps: OnboardingStep[];
  index: number;
  onPrev: () => void;
  onNext: () => void;
  onSkip: () => void;
  onClose: () => void;
}) {
  const step = steps[index] ?? null;
  const progress = onboardingProgress(index, steps.length);
  const isLast = index >= steps.length - 1;
  const [rect, setRect] = useState<DOMRect | null>(null);
  const [anchorMissing, setAnchorMissing] = useState(false);

  useEffect(() => {
    if (!step) {
      return;
    }
    let frame = 0;
    const measure = () => {
      const target = document.querySelector(`[data-testid="${step.anchorTestId}"]`);
      if (!target) {
        setRect(null);
        setAnchorMissing(true);
        return;
      }
      setRect(target.getBoundingClientRect());
      setAnchorMissing(false);
    };
    measure();
    const schedule = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(measure);
    };
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("resize", schedule);
      window.removeEventListener("scroll", schedule, true);
    };
  }, [step]);

  if (!step) {
    return null;
  }

  return (
    <>
      {rect ? (
        <div
          data-testid="onboarding-highlight"
          aria-hidden="true"
          className="pointer-events-none fixed z-40 rounded-lg ring-2 ring-[var(--aria-primary)] transition-all duration-200 motion-reduce:transition-none"
          style={{
            top: rect.top - HIGHLIGHT_PADDING,
            left: rect.left - HIGHLIGHT_PADDING,
            width: rect.width + HIGHLIGHT_PADDING * 2,
            height: rect.height + HIGHLIGHT_PADDING * 2,
            boxShadow: "0 0 0 9999px rgba(15, 23, 42, 0.35)",
          }}
        />
      ) : null}
      <section
        data-testid="onboarding-wizard"
        role="dialog"
        aria-label="操作引导"
        aria-modal="false"
        className="fixed bottom-4 left-1/2 z-50 w-[min(26rem,calc(100vw-2rem))] -translate-x-1/2 rounded-2xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-[0_24px_64px_rgba(15,23,42,0.22)]"
      >
        <header className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <p
              data-testid="onboarding-progress"
              className="text-xs font-semibold text-[var(--aria-primary)]"
            >
              {`第 ${progress.current} / 共 ${progress.total} 步`}
            </p>
            <h2
              data-testid="onboarding-step-title"
              className="mt-1 text-sm font-semibold text-[var(--aria-ink)]"
            >
              {step.title}
            </h2>
          </div>
          <button
            type="button"
            data-testid="onboarding-close"
            aria-label="关闭引导"
            onClick={onClose}
            className="inline-flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-lg border border-[var(--aria-line)] text-[var(--aria-ink-muted)] transition-colors hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            <X aria-hidden="true" className="h-4 w-4" />
          </button>
        </header>

        <p
          data-testid="onboarding-step-description"
          className="mt-2 text-sm leading-6 text-[var(--aria-ink-muted)]"
        >
          {step.description}
        </p>

        <div
          aria-label={step.visual.alt}
          className="mt-3 rounded-lg border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2"
        >
          {step.visual.kind === "screenshot" && step.visual.src ? (
            <img src={step.visual.src} alt={step.visual.alt} className="w-full rounded-md" />
          ) : (
            <p className="text-xs text-[var(--aria-ink-muted)]">{step.visual.alt}</p>
          )}
        </div>

        {anchorMissing ? (
          <p
            data-testid="onboarding-anchor-missing"
            role="status"
            className="mt-2 text-xs text-[var(--aria-ink-muted)]"
          >
            当前区域尚未出现，完成前置步骤后继续。
          </p>
        ) : null}

        <footer className="mt-4 flex items-center justify-between gap-2">
          <button
            type="button"
            data-testid="onboarding-skip"
            onClick={onSkip}
            className="cursor-pointer rounded-lg px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)] transition-colors hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            跳过
          </button>
          <div className="flex items-center gap-2">
            <button
              type="button"
              data-testid="onboarding-prev"
              disabled={index <= 0}
              onClick={onPrev}
              className="inline-flex h-8 cursor-pointer items-center gap-1 rounded-lg border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel-muted)] disabled:cursor-not-allowed disabled:opacity-40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
            >
              <ArrowLeft aria-hidden="true" className="h-3.5 w-3.5" />
              上一步
            </button>
            {isLast ? (
              <button
                type="button"
                data-testid="onboarding-finish"
                onClick={onClose}
                className="inline-flex h-8 cursor-pointer items-center gap-1 rounded-lg border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
              >
                <Check aria-hidden="true" className="h-3.5 w-3.5" />
                完成
              </button>
            ) : (
              <button
                type="button"
                data-testid="onboarding-next"
                onClick={onNext}
                className="inline-flex h-8 cursor-pointer items-center gap-1 rounded-lg border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
              >
                下一步
                <ArrowRight aria-hidden="true" className="h-3.5 w-3.5" />
              </button>
            )}
          </div>
        </footer>
      </section>
    </>
  );
}
