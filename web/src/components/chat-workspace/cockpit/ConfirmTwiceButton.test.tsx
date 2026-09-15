import { createRef } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ConfirmTwiceButtonHandle } from "../../../state/cockpit-operation-semantics";
import { ConfirmTwiceButton } from "./ConfirmTwiceButton";

describe("ConfirmTwiceButton", () => {
  it("arms once, falls back after ten seconds, and only then calls onConfirm", () => {
    vi.useFakeTimers();
    const onConfirm = vi.fn();
    render(<ConfirmTwiceButton label="终止" confirmLabel="确认终止" onConfirm={onConfirm} />);

    fireEvent.click(screen.getByRole("button", { name: "终止" }));
    expect(screen.getByRole("button", { name: "确认终止" })).toBeVisible();
    act(() => {
      vi.advanceTimersByTime(10_000);
    });
    expect(screen.getByRole("button", { name: "终止" })).toBeVisible();
    expect(onConfirm).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "终止" }));
    fireEvent.click(screen.getByRole("button", { name: "确认终止" }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });

  it("exposes arm through ref and confirms only on the second arm", () => {
    const ref = createRef<ConfirmTwiceButtonHandle>();
    const onConfirm = vi.fn();
    render(<ConfirmTwiceButton ref={ref} label="接管" confirmLabel="确认接管" onConfirm={onConfirm} />);

    act(() => {
      ref.current?.arm();
    });
    expect(screen.getByRole("button", { name: "确认接管" })).toBeVisible();
    expect(onConfirm).not.toHaveBeenCalled();
    act(() => {
      ref.current?.arm();
    });
    expect(onConfirm).toHaveBeenCalledOnce();
  });
});
