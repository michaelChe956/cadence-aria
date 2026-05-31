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

function installLocalStorageMock() {
  const store = new Map<string, string>();
  const storage: Storage = {
    get length() {
      return store.size;
    },
    clear: vi.fn(() => store.clear()),
    getItem: vi.fn((key: string) => store.get(key) ?? null),
    key: vi.fn((index: number) => Array.from(store.keys())[index] ?? null),
    removeItem: vi.fn((key: string) => {
      store.delete(key);
    }),
    setItem: vi.fn((key: string, value: string) => {
      store.set(key, value);
    }),
  };
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: storage,
  });
}

beforeEach(() => {
  installResizeObserverMock();
  installScrollToMock();
  installLocalStorageMock();
});
installResizeObserverMock();
installScrollToMock();
installLocalStorageMock();

afterEach(() => cleanup());
