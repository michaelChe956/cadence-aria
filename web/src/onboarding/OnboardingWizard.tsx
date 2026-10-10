import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ArrowLeft, ArrowRight, Check, X } from "lucide-react";
import { onboardingProgress, type OnboardingStep } from "./steps";
import {
  computeCardPlacement,
  type CardPlacement,
  type PlacementRect,
} from "./placement";

const HIGHLIGHT_PADDING = 4;
// 回退定位类与 placement.ts 的 FALLBACK_BOTTOM_OFFSET(72) 保持一致：
// 抬高避开左下「操作引导」与右下「图片创作」两个 fixed 悬浮按钮。
const FALLBACK_POSITION_CLASS = "bottom-[72px] left-1/2 -translate-x-1/2";

/**
 * 无第三方依赖的引导浮层：说明卡 + 位置/总数 + 上一步/下一步/跳过/完成 +
 * 示意资源降级 + 缺失锚点提示 + 视觉高亮。
 *
 * 高亮层 `pointer-events-none`，不捕获业务点击；说明卡提供非阻塞控制。
 * 说明卡与高亮层经 portal 挂 document.body——fixed 恒以视口为包含块，
 * 免疫 transform/filter 祖先劫持（同 DialogShell 先例，fc0963ef 根因）。
 * 有锚点时卡片定位到目标旁（右/下/左/上自适应翻转 + 视口夹紧，见 placement.ts），
 * 指向标记对准目标中心；无锚点回退视口底部居中。
 * 步骤切换 / resize / scroll / 卡片内容尺寸变化时重新测量，卸载时清理监听与高亮。
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
  const [targetRect, setTargetRect] = useState<PlacementRect | null>(null);
  const [anchorMissing, setAnchorMissing] = useState(false);
  const [placement, setPlacement] = useState<CardPlacement | null>(null);
  const cardRef = useRef<HTMLElement | null>(null);

  useLayoutEffect(() => {
    if (!step) {
      return;
    }
    const selector = `[data-testid="${step.anchorTestId}"]`;
    // 先尽量把目标滚进可视区再测量（jsdom 未实现 scrollIntoView，可选调用跳过）。
    document.querySelector(selector)?.scrollIntoView?.({ block: "center", inline: "nearest" });

    let frame = 0;
    const measure = () => {
      const target = document.querySelector(selector);
      if (!target) {
        setTargetRect(null);
        setAnchorMissing(true);
        setPlacement(null);
        return;
      }
      const box = target.getBoundingClientRect();
      const rect: PlacementRect = {
        top: box.top,
        left: box.left,
        width: box.width,
        height: box.height,
      };
      setTargetRect((prev) =>
        prev && prev.top === rect.top && prev.left === rect.left &&
          prev.width === rect.width && prev.height === rect.height
          ? prev
          : rect,
      );
      setAnchorMissing(false);
      const cardBox = cardRef.current?.getBoundingClientRect();
      const next = computeCardPlacement({
        target: rect,
        viewport: { width: window.innerWidth, height: window.innerHeight },
        cardSize: cardBox
          ? { width: cardBox.width, height: cardBox.height }
          : { width: 0, height: 0 },
      });
      setPlacement((prev) =>
        prev && prev.top === next.top && prev.left === next.left && prev.side === next.side &&
          prev.arrow?.top === next.arrow?.top && prev.arrow?.left === next.arrow?.left
          ? prev
          : next,
      );
    };
    // 初始同步测量（提交后、绘制前），避免回退位闪现。
    measure();
    const schedule = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(measure);
    };
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
    // 示意截图加载完成等引起卡片高度变化时重算（测试环境 setup.ts 已装 mock）。
    const cardElement = cardRef.current;
    const observer =
      cardElement && typeof ResizeObserver !== "undefined"
        ? new ResizeObserver(schedule)
        : null;
    if (observer && cardElement) {
      observer.observe(cardElement);
    }
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("resize", schedule);
      window.removeEventListener("scroll", schedule, true);
      observer?.disconnect();
    };
  }, [step]);

  // 非模态引导卡：Esc 与关闭按钮同路；刻意不锁 body/不圈闭焦点（引导需与页面交互）。
  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  if (!step || typeof document === "undefined") {
    return null;
  }

  const anchored = placement !== null;

  return createPortal(
    <>
      {targetRect ? (
        <div
          data-testid="onboarding-highlight"
          aria-hidden="true"
          className="pointer-events-none fixed z-40 rounded-lg ring-2 ring-[var(--aria-primary)] transition-all duration-200 motion-reduce:transition-none"
          style={{
            top: targetRect.top - HIGHLIGHT_PADDING,
            left: targetRect.left - HIGHLIGHT_PADDING,
            width: targetRect.width + HIGHLIGHT_PADDING * 2,
            height: targetRect.height + HIGHLIGHT_PADDING * 2,
            boxShadow: "0 0 0 9999px rgba(15, 23, 42, 0.35)",
          }}
        />
      ) : null}
      <section
        ref={cardRef}
        data-testid="onboarding-wizard"
        data-anchor-side={placement?.side ?? "fallback"}
        role="dialog"
        aria-label="操作引导"
        aria-modal="false"
        className={`fixed z-50 w-[min(26rem,calc(100vw-2rem))] rounded-2xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-[0_24px_64px_rgba(15,23,42,0.22)]${anchored ? "" : ` ${FALLBACK_POSITION_CLASS}`}`}
        style={anchored ? { top: placement.top, left: placement.left } : undefined}
      >
        {placement?.arrow ? (
          <span
            data-testid="onboarding-arrow"
            aria-hidden="true"
            className="absolute h-3 w-3 rotate-45 border border-[var(--aria-line)] bg-[var(--aria-panel)]"
            style={{ top: placement.arrow.top - 6, left: placement.arrow.left - 6 }}
          />
        ) : null}
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
    </>,
    document.body,
  );
}
