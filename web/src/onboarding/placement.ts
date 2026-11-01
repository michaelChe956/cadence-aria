/**
 * 引导卡定位纯函数：给定目标矩形、视口与卡片尺寸，计算卡片左上角视口坐标与
 * 指向标记位置。无 DOM 依赖，OnboardingWizard 测量后调用，可独立单测。
 *
 * 规则：优先目标右侧，其次下方、左侧、上方；候选位置先夹回视口安全边距内，
 * 再校验与目标矩形的遮挡（重叠面积须为 0）；四向都遮挡时取重叠最小的一侧
 * （同重叠按偏好序），保证卡片永不溢出视口、正常情况下不遮目标。
 */

export type PlacementRect = { top: number; left: number; width: number; height: number };

export type CardSide = "right" | "bottom" | "left" | "top";

/** 指向标记中心（卡片本地坐标，相对卡片左上角）。 */
export type CardArrow = { top: number; left: number };

export type CardPlacement = {
  /** 卡片左上角视口坐标，可直接用于 fixed 定位。 */
  top: number;
  left: number;
  /** 卡片相对目标的边；null = 无目标回退（视口底部居中）。 */
  side: CardSide | null;
  /** 指向标记中心（卡片本地坐标）；side 为 null 时无。 */
  arrow: CardArrow | null;
};

/** 卡片与目标之间的间距。 */
export const CARD_GAP = 12;
/** 视口安全边距：卡片任意边不越过。 */
export const CARD_VIEWPORT_PADDING = 12;
/** 无目标回退时卡片距视口底部的偏移（避开左下/右下两个 fixed 悬浮按钮）。 */
export const FALLBACK_BOTTOM_OFFSET = 72;
/** 指向标记中心距卡片边界的最小距离（防止箭头贴角出界）。 */
const ARROW_EDGE_MARGIN = 10;

/** 侧向偏好序：右 → 下 → 左 → 上。 */
const SIDE_PREFERENCE: readonly CardSide[] = ["right", "bottom", "left", "top"];

type Viewport = { width: number; height: number };
type Size = { width: number; height: number };

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(value, max));
}

function overlapArea(a: PlacementRect, b: PlacementRect): number {
  const x = Math.min(a.left + a.width, b.left + b.width) - Math.max(a.left, b.left);
  const y = Math.min(a.top + a.height, b.top + b.height) - Math.max(a.top, b.top);
  return x > 0 && y > 0 ? x * y : 0;
}

/** 各侧向的未夹紧理想位置（卡片左上角）。 */
function candidateOrigin(
  side: CardSide,
  target: PlacementRect,
  cardSize: Size,
): { top: number; left: number } {
  const targetCenterX = target.left + target.width / 2;
  const targetCenterY = target.top + target.height / 2;
  switch (side) {
    case "right":
      return {
        left: target.left + target.width + CARD_GAP,
        top: targetCenterY - cardSize.height / 2,
      };
    case "left":
      return {
        left: target.left - CARD_GAP - cardSize.width,
        top: targetCenterY - cardSize.height / 2,
      };
    case "bottom":
      return {
        left: targetCenterX - cardSize.width / 2,
        top: target.top + target.height + CARD_GAP,
      };
    case "top":
      return {
        left: targetCenterX - cardSize.width / 2,
        top: target.top - CARD_GAP - cardSize.height,
      };
  }
}

/** 指向标记中心：贴住卡片朝向目标的那条边，并对准目标中心（沿边夹紧）。 */
function arrowFor(
  side: CardSide,
  target: PlacementRect,
  cardSize: Size,
  cardTop: number,
  cardLeft: number,
): CardArrow {
  const targetCenterX = target.left + target.width / 2;
  const targetCenterY = target.top + target.height / 2;
  switch (side) {
    case "right":
      return {
        left: 0,
        top: clamp(targetCenterY - cardTop, ARROW_EDGE_MARGIN, cardSize.height - ARROW_EDGE_MARGIN),
      };
    case "left":
      return {
        left: cardSize.width,
        top: clamp(targetCenterY - cardTop, ARROW_EDGE_MARGIN, cardSize.height - ARROW_EDGE_MARGIN),
      };
    case "bottom":
      return {
        top: 0,
        left: clamp(targetCenterX - cardLeft, ARROW_EDGE_MARGIN, cardSize.width - ARROW_EDGE_MARGIN),
      };
    case "top":
      return {
        top: cardSize.height,
        left: clamp(targetCenterX - cardLeft, ARROW_EDGE_MARGIN, cardSize.width - ARROW_EDGE_MARGIN),
      };
  }
}

export function computeCardPlacement(input: {
  target: PlacementRect | null;
  viewport: Viewport;
  cardSize: Size;
}): CardPlacement {
  const { target, viewport, cardSize } = input;

  // 无目标回退：视口底部居中，抬高避开左下/右下 fixed 悬浮按钮，仍夹在安全边距内。
  if (!target) {
    const left = clamp(
      (viewport.width - cardSize.width) / 2,
      CARD_VIEWPORT_PADDING,
      viewport.width - CARD_VIEWPORT_PADDING - cardSize.width,
    );
    const top = clamp(
      viewport.height - cardSize.height - FALLBACK_BOTTOM_OFFSET,
      CARD_VIEWPORT_PADDING,
      viewport.height - CARD_VIEWPORT_PADDING - cardSize.height,
    );
    return { top, left, side: null, arrow: null };
  }

  let best: { side: CardSide; top: number; left: number; overlap: number } | null = null;
  for (const side of SIDE_PREFERENCE) {
    const origin = candidateOrigin(side, target, cardSize);
    const left = clamp(
      origin.left,
      CARD_VIEWPORT_PADDING,
      viewport.width - CARD_VIEWPORT_PADDING - cardSize.width,
    );
    const top = clamp(
      origin.top,
      CARD_VIEWPORT_PADDING,
      viewport.height - CARD_VIEWPORT_PADDING - cardSize.height,
    );
    const overlap = overlapArea(
      { top, left, width: cardSize.width, height: cardSize.height },
      target,
    );
    if (overlap === 0) {
      return { top, left, side, arrow: arrowFor(side, target, cardSize, top, left) };
    }
    if (!best || overlap < best.overlap) {
      best = { side, top, left, overlap };
    }
  }

  // 视口装不下任何不遮挡位（目标过大/视口过小）：取重叠最小的一侧，卡片仍在视口内。
  const chosen = best as { side: CardSide; top: number; left: number };
  return {
    top: chosen.top,
    left: chosen.left,
    side: chosen.side,
    arrow: arrowFor(chosen.side, target, cardSize, chosen.top, chosen.left),
  };
}
