import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ChangelogEntry } from "../../whats-new/changelog";
import { WhatsNewDialog } from "./WhatsNewDialog";

const entries: ChangelogEntry[] = [
  {
    version: "0.0.8",
    date: "2026-08-17",
    title: "v0.0.8 更新",
    highlights: ["0.0.8 要点"],
  },
  {
    version: "0.0.7",
    date: "2026-08-13",
    title: "v0.0.7 更新",
    highlights: ["0.0.7 要点"],
  },
  {
    version: "0.0.6",
    date: "2026-08-08",
    title: "v0.0.6 更新",
    highlights: ["0.0.6 要点"],
  },
  {
    version: "0.0.5",
    date: "2026-08-08",
    title: "v0.0.5 更新",
    highlights: ["要点一", "要点二"],
  },
];

describe("WhatsNewDialog", () => {
  it("按新到旧展示四个版本区块的标题、日期与要点", () => {
    render(<WhatsNewDialog entries={entries} onClose={() => {}} />);
    const dialog = screen.getByRole("dialog", { name: "版本更新说明" });
    const sections = screen.getAllByRole("region");

    expect(sections).toHaveLength(4);
    expect(sections.map((section) => section.getAttribute("aria-label"))).toEqual([
      "0.0.8 · 2026-08-17",
      "0.0.7 · 2026-08-13",
      "0.0.6 · 2026-08-08",
      "0.0.5 · 2026-08-08",
    ]);
    for (const [index, entry] of entries.entries()) {
      expect(sections[index]).toHaveTextContent(entry.title);
      expect(sections[index]).toHaveTextContent(entry.date);
      for (const highlight of entry.highlights) {
        expect(sections[index]).toHaveTextContent(highlight);
      }
    }
  });

  it("点击知道了按钮触发 onClose", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<WhatsNewDialog entries={entries} onClose={onClose} />);
    await user.click(screen.getByRole("button", { name: "知道了" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
