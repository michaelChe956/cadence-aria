import { describe, expect, it } from "vitest";
import {
  CARD_GAP,
  CARD_VIEWPORT_PADDING,
  FALLBACK_BOTTOM_OFFSET,
  computeCardPlacement,
  type CardPlacement,
} from "./placement";

const wide = { width: 1280, height: 800 };
const card = { width: 400, height: 300 };

function cardRect(placement: CardPlacement) {
  return {
    top: placement.top,
    left: placement.left,
    width: card.width,
    height: card.height,
  };
}

function overlapArea(
  a: { top: number; left: number; width: number; height: number },
  b: { top: number; left: number; width: number; height: number },
): number {
  const x = Math.min(a.left + a.width, b.left + b.width) - Math.max(a.left, b.left);
  const y = Math.min(a.top + a.height, b.top + b.height) - Math.max(a.top, b.top);
  return x > 0 && y > 0 ? x * y : 0;
}

describe("computeCardPlacement", () => {
  it("无目标时回退到视口底部居中（避开左下/右下悬浮按钮）", () => {
    const placement = computeCardPlacement({ target: null, viewport: wide, cardSize: card });
    expect(placement.side).toBeNull();
    expect(placement.arrow).toBeNull();
    expect(placement.left).toBe((1280 - 400) / 2);
    expect(placement.top).toBe(800 - 300 - FALLBACK_BOTTOM_OFFSET);
  });

  it("无目标且卡片超宽时回退位置仍夹在视口内", () => {
    const placement = computeCardPlacement({
      target: null,
      viewport: { width: 500, height: 400 },
      cardSize: { width: 600, height: 300 },
    });
    expect(placement.left).toBe(CARD_VIEWPORT_PADDING);
    expect(placement.top).toBe(400 - 300 - FALLBACK_BOTTOM_OFFSET);
  });

  it("四周开阔时优先放目标右侧，指向标记对准目标中心", () => {
    const placement = computeCardPlacement({
      target: { top: 100, left: 100, width: 200, height: 60 },
      viewport: wide,
      cardSize: card,
    });
    expect(placement.side).toBe("right");
    expect(placement.left).toBe(100 + 200 + CARD_GAP);
    // 垂直居中被视口上边距夹紧
    expect(placement.top).toBe(CARD_VIEWPORT_PADDING);
    expect(placement.arrow).toEqual({ left: 0, top: 118 });
    expect(overlapArea(cardRect(placement), { top: 100, left: 100, width: 200, height: 60 })).toBe(
      0,
    );
  });

  it("右侧放不下（目标贴右缘）时翻到下方", () => {
    const target = { top: 100, left: 980, width: 200, height: 60 };
    const placement = computeCardPlacement({ target, viewport: wide, cardSize: card });
    expect(placement.side).toBe("bottom");
    expect(placement.top).toBe(100 + 60 + CARD_GAP);
    // 期望居中被右边距夹紧
    expect(placement.left).toBe(1280 - CARD_VIEWPORT_PADDING - 400);
    expect(placement.arrow).toEqual({ top: 0, left: 212 });
    expect(overlapArea(cardRect(placement), target)).toBe(0);
  });

  it("右、下都放不下时翻到左侧", () => {
    const target = { top: 680, left: 900, width: 200, height: 60 };
    const placement = computeCardPlacement({
      target,
      viewport: wide,
      cardSize: { width: 200, height: 120 },
    });
    expect(placement.side).toBe("left");
    expect(placement.left).toBe(900 - CARD_GAP - 200);
    expect(placement.top).toBe(710 - 60);
    expect(placement.arrow).toEqual({ left: 200, top: 60 });
  });

  it("右、下、左都放不下时翻到上方", () => {
    const target = { top: 640, left: 250, width: 200, height: 60 };
    const placement = computeCardPlacement({
      target,
      viewport: { width: 700, height: 800 },
      cardSize: card,
    });
    expect(placement.side).toBe("top");
    expect(placement.top).toBe(640 - CARD_GAP - 300);
    expect(placement.left).toBe(350 - 200);
    expect(placement.arrow).toEqual({ top: 300, left: 200 });
    expect(overlapArea(cardRect(placement), target)).toBe(0);
  });

  it("卡片高于视口时贴边距夹紧，不横向溢出", () => {
    const placement = computeCardPlacement({
      target: { top: 50, left: 100, width: 200, height: 60 },
      viewport: { width: 1280, height: 600 },
      cardSize: { width: 400, height: 900 },
    });
    expect(placement.side).toBe("right");
    expect(placement.top).toBe(CARD_VIEWPORT_PADDING);
    expect(placement.left).toBe(100 + 200 + CARD_GAP);
  });

  it("极端拥挤（四向都遮目标）时按偏好序取最小重叠并保持不出视口", () => {
    const target = { top: 150, left: 200, width: 200, height: 100 };
    const placement = computeCardPlacement({
      target,
      viewport: { width: 600, height: 400 },
      cardSize: card,
    });
    expect(placement.side).toBe("right");
    expect(placement.left).toBe(600 - CARD_VIEWPORT_PADDING - 400);
    expect(placement.top).toBe(50);
    // 卡片整体仍在视口内
    expect(placement.top).toBeGreaterThanOrEqual(CARD_VIEWPORT_PADDING);
    expect(placement.left).toBeGreaterThanOrEqual(CARD_VIEWPORT_PADDING);
    expect(placement.left + 400).toBeLessThanOrEqual(600 - CARD_VIEWPORT_PADDING);
  });

  it("指向标记沿卡片边缘夹紧，不超出卡片", () => {
    const placement = computeCardPlacement({
      target: { top: 600, left: 440, width: 400, height: 60 },
      viewport: wide,
      cardSize: card,
    });
    expect(placement.side).toBe("right");
    expect(placement.arrow).toEqual({ left: 0, top: 150 });
  });
});
