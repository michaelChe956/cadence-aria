// 2026-11-05（owner 真机事故回归）：侧栏垃圾桶一击即硬删整个项目工作区
//（项目 + 全部逻辑代码库 + 成员登记 + capability 记录）且无任何确认。
// 修复形态对齐逻辑代码库删除/清除的二次确认：点击先弹确认——取消零副作用
//（零出站删除请求）；确认才发带确认意图的 DELETE ?confirmed=true（后端
// 409 deletion_confirmation_required 强制兜底，不靠前端自律）。
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  installIssueLifecycleWorkbenchTestHooks,
  lifecycleFetch,
  projectRecord,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => (
    <div data-testid="monaco-viewer">{value}</div>
  ),
}));

type FetchCall = { method: string; url: string };

function installFetch(): FetchCall[] {
  const calls: FetchCall[] = [];
  const fetchMock = lifecycleFetch({
    projects: [projectRecord("project_0001", "Aria")],
  });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push({ method: init?.method ?? "GET", url: String(input) });
      return fetchMock(input, init);
    }),
  );
  return calls;
}

describe("IssueLifecycleWorkbench 删除 Project 二次确认（2026-11-05）", () => {
  installIssueLifecycleWorkbenchTestHooks();
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("取消确认零副作用：不发删除请求，Project 保留", async () => {
    const calls = installFetch();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "删除 Project Aria" }),
    );

    expect(confirmSpy).toHaveBeenCalledTimes(1);
    // 文案必须说清删除范围（整个项目及其全部逻辑代码库/成员登记）与不可恢复。
    const message = confirmSpy.mock.calls[0]?.[0] ?? "";
    expect(message).toContain("不可恢复");
    expect(message).toContain("逻辑代码库");
    expect(message).toContain("成员登记");
    // 取消 ⇒ 零出站删除请求。
    expect(calls.filter((call) => call.method === "DELETE")).toHaveLength(0);
    // Project 仍在侧栏。
    expect(screen.getByRole("button", { name: "Aria" })).toBeInTheDocument();
  });

  it("确认后发带确认意图的删除请求并刷新列表", async () => {
    const calls = installFetch();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(
      await screen.findByRole("button", { name: "删除 Project Aria" }),
    );

    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(calls).toContainEqual({
      method: "DELETE",
      url: "/api/projects/project_0001?confirmed=true",
    });
    expect(await screen.findByText("还没有 Project")).toBeInTheDocument();
  });
});
