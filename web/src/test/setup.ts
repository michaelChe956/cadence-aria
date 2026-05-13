import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, beforeEach, vi } from "vitest";

class ResizeObserverMock {
  private readonly callback: ResizeObserverCallback;

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
  }

  observe = vi.fn((target: Element) => {
    this.callback(
      [
        {
          target,
          contentRect: {
            x: 0,
            y: 0,
            width: 1440,
            height: 960,
            top: 0,
            left: 0,
            right: 1440,
            bottom: 960,
            toJSON() {
              return this;
            },
          } as DOMRectReadOnly,
        } as ResizeObserverEntry,
      ],
      this as unknown as ResizeObserver,
    );
  });

  unobserve = vi.fn();
  disconnect = vi.fn();
}

function installResizeObserverMock() {
  Object.defineProperty(globalThis, "ResizeObserver", {
    configurable: true,
    value: ResizeObserverMock,
  });
  Object.defineProperty(window, "ResizeObserver", {
    configurable: true,
    value: ResizeObserverMock,
  });
}

function installScrollToMock() {
  Object.defineProperty(window, "scrollTo", {
    configurable: true,
    value: vi.fn(),
  });
}

beforeEach(() => {
  installResizeObserverMock();
  installScrollToMock();
});
installResizeObserverMock();
installScrollToMock();

afterEach(() => cleanup());
