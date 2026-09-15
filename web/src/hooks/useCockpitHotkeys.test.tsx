import { fireEvent, render, screen } from "@testing-library/react";
import { type ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import {
  useCockpitHotkeys,
} from "./useCockpitHotkeys";
import type { CockpitHotkeyHandlers } from "../state/cockpit-operation-semantics";

function HotkeyHarness({ handlers }: { handlers: CockpitHotkeyHandlers }): ReactNode {
  useCockpitHotkeys(handlers);
  return (
    <>
      <input aria-label="可编辑输入" />
      <select aria-label="可编辑选择">
        <option>选项</option>
      </select>
    </>
  );
}

describe("useCockpitHotkeys", () => {
  it("dispatches each shared chord only outside editable controls", () => {
    const handlers = {
      confirm: vi.fn(),
      feedback: vi.fn(),
      takeover: vi.fn(),
      advance: vi.fn(),
    };
    render(<HotkeyHarness handlers={handlers} />);

    fireEvent.keyDown(document, { code: "Enter", ctrlKey: true });
    fireEvent.keyDown(document, { code: "KeyF", ctrlKey: true });
    fireEvent.keyDown(document, { code: "KeyT", ctrlKey: true, shiftKey: true });
    fireEvent.keyDown(document, { code: "KeyA", ctrlKey: true });

    expect(handlers.confirm).toHaveBeenCalledOnce();
    expect(handlers.feedback).toHaveBeenCalledOnce();
    expect(handlers.takeover).toHaveBeenCalledOnce();
    expect(handlers.advance).toHaveBeenCalledOnce();

    fireEvent.keyDown(screen.getByRole("textbox"), { code: "KeyA", ctrlKey: true });
    fireEvent.keyDown(screen.getByRole("combobox"), { code: "Enter", ctrlKey: true });

    expect(handlers.confirm).toHaveBeenCalledOnce();
    expect(handlers.advance).toHaveBeenCalledOnce();
  });

  it("ignores prevented and Alt-modified shared chords", () => {
    const handlers = {
      confirm: vi.fn(),
      feedback: vi.fn(),
      takeover: vi.fn(),
      advance: vi.fn(),
    };
    render(<HotkeyHarness handlers={handlers} />);
    const prevented = new KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      code: "Enter",
      ctrlKey: true,
    });
    prevented.preventDefault();
    document.dispatchEvent(prevented);
    fireEvent.keyDown(document, { code: "Enter", ctrlKey: true, altKey: true });

    expect(handlers.confirm).not.toHaveBeenCalled();
  });
});
