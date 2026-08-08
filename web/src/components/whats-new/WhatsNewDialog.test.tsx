import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ChangelogEntry } from "../../whats-new/changelog";
import { WhatsNewDialog } from "./WhatsNewDialog";

const entry: ChangelogEntry = {
  version: "0.0.5",
  date: "2026-08-08",
  title: "v0.0.5 更新",
  highlights: ["要点一", "要点二"],
};

describe("WhatsNewDialog", () => {
  it("展示版本标题、日期与要点列表", () => {
    render(<WhatsNewDialog entry={entry} onClose={() => {}} />);
    const dialog = screen.getByRole("dialog", { name: "版本更新说明" });
    expect(dialog).toHaveTextContent("v0.0.5 更新");
    expect(dialog).toHaveTextContent("2026-08-08");
    expect(dialog).toHaveTextContent("要点一");
    expect(dialog).toHaveTextContent("要点二");
  });

  it("点击知道了按钮触发 onClose", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<WhatsNewDialog entry={entry} onClose={onClose} />);
    await user.click(screen.getByRole("button", { name: "知道了" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
