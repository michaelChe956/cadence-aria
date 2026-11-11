import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MarkdownContent } from "./MarkdownContent";

describe("MarkdownContent", () => {
  it("渲染标题与列表", () => {
    render(<MarkdownContent content="# 知识库标题\n\n- 领域一\n- 领域二" />);

    expect(
      screen.getByRole("heading", { level: 1, name: "知识库标题" }),
    ).toBeInTheDocument();
    const items = screen.getAllByRole("listitem");
    expect(items.map((item) => item.textContent)).toEqual(["领域一", "领域二"]);
  });

  it("javascript: 链接被白名单过滤为纯文本（与迁移前同断言）", () => {
    render(
      <MarkdownContent
        content={"[危险链接](javascript:alert(1))\n\n[安全链接](https://example.com/kb)"}
      />,
    );

    const links = screen.getAllByRole("link");
    expect(links).toHaveLength(1);
    expect(links[0]).toHaveAttribute("href", "https://example.com/kb");
    expect(links[0].textContent).toBe("安全链接");
    expect(screen.getByText("危险链接").closest("a")).toBeNull();
  });

  it("空内容渲染为空容器（迁移前行为）", () => {
    const { container } = render(<MarkdownContent content="" />);
    expect(container.textContent).toBe("");
  });
});
