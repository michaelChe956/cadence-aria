import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LogicalCodebaseRegistrationWizard } from "./LogicalCodebaseRegistrationWizard";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function registrationFetch(preflightItems?: unknown[]) {
  const submit = { batch_id: "batch_0001", status: "partial_failed", items: [
    { path: "/root/api", status: "completed", failure_reason: null },
    { path: "/root/web", status: "failed", failure_reason: "dirty" },
  ] };
  const resume = { ...submit, status: "completed", items: submit.items.map((item) => ({ ...item, status: "completed", failure_reason: null })) };
  return vi.fn(async (input: RequestInfo | URL, _init?: RequestInit) => {
    const path = String(input);
    if (path.endsWith("/preflight")) {
      return jsonResponse({ preflight_id: "preflight_0001", created_at: "2026-08-18T00:00:00Z", items: preflightItems ?? [
        { path: "/root/api", class: "eligible", reason: null },
        { path: "/root/web", class: "needs_attention", reason: "dirty" },
        { path: "/root/non-git", class: "non_git", reason: "not_git" },
        { path: "/root/duplicate", class: "duplicate", reason: "duplicate" },
        { path: "/root/nested", class: "nested", reason: "nested" },
        { path: "/root/missing", class: "missing", reason: "missing" },
        { path: "/tmp/outside", class: "outside_root", reason: "outside_root" },
      ] });
    }
    if (path.endsWith("/resume")) return jsonResponse(resume);
    return jsonResponse(submit);
  });
}

describe("LogicalCodebaseRegistrationWizard", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("自动拉取候选、按分类初始化勾选，并只提交勾选项", async () => {
    const fetchMock = registrationFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onCompleted = vi.fn();
    render(<LogicalCodebaseRegistrationWizard projectId="project_0001" onCompleted={onCompleted} onClose={vi.fn()} />);

    await user.type(screen.getByLabelText("聚合根目录"), "/root");
    await user.click(screen.getByRole("button", { name: "确认聚合根并自动发现" }));

    expect(await screen.findByText("needs_attention")).toBeInTheDocument();
    expect(screen.getByText("non_git")).toBeInTheDocument();
    expect(screen.getByText("outside_root")).toBeInTheDocument();
    const eligible = screen.getByLabelText("选择 /root/api（eligible）");
    expect(eligible).toBeChecked();
    const dirty = screen.getByLabelText("确认 /root/web（dirty）");
    expect(dirty).not.toBeChecked();
    const nonGit = screen.getByLabelText("不可登记 /root/non-git（not_git）");
    expect(nonGit).not.toBeChecked();
    expect(nonGit).toBeDisabled();
    await user.click(eligible);
    await user.click(dirty);
    await user.click(screen.getByRole("button", { name: "提交登记" }));

    const submitCall = fetchMock.mock.calls.find(([input]) =>
      String(input).endsWith("/logical-codebase/registrations"),
    );
    expect(JSON.parse(String(submitCall?.[1]?.body))).toMatchObject({
      confirmed_paths: ["/root/web"],
    });
    expect(await screen.findByText("partial_failed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "恢复未完成项" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "恢复未完成项" }));
    expect(await screen.findByText("completed")).toBeInTheDocument();
    expect(onCompleted).toHaveBeenCalledTimes(1);

    const preflightCall = fetchMock.mock.calls.find(([input]) => String(input).endsWith("/preflight"));
    expect(JSON.parse(String(preflightCall?.[1]?.body))).toEqual({
      aggregate_root: "/root",
      candidate_paths: [],
      auto_discover: true,
    });
  });

  it("needs_attention 未勾选时不混入 confirmed_paths", async () => {
    const fetchMock = registrationFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    render(<LogicalCodebaseRegistrationWizard projectId="project_0001" onCompleted={vi.fn()} onClose={vi.fn()} />);

    await user.type(screen.getByLabelText("聚合根目录"), "/root");
    await user.click(screen.getByRole("button", { name: "确认聚合根并自动发现" }));
    await user.click(screen.getByRole("button", { name: "提交登记" }));

    const submitCall = fetchMock.mock.calls.find(([input]) =>
      String(input).endsWith("/logical-codebase/registrations"),
    );
    expect(JSON.parse(String(submitCall?.[1]?.body))).toMatchObject({
      confirmed_paths: ["/root/api"],
    });
  });

  it("自动发现失败后保留手工预检兜底", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => jsonResponse({ code: "aggregate_root_missing", message: "聚合根不存在" }, 422)),
    );
    const user = userEvent.setup();
    render(<LogicalCodebaseRegistrationWizard projectId="project_0001" onCompleted={vi.fn()} onClose={vi.fn()} />);

    await user.type(screen.getByLabelText("聚合根目录"), "/missing");
    await user.click(screen.getByRole("button", { name: "确认聚合根并自动发现" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("聚合根不存在");
    expect(screen.getByLabelText("候选成员路径")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "执行手工预检" })).toBeInTheDocument();
  });

  it("自动发现为空时保留手工预检兜底", async () => {
    const fetchMock = registrationFetch([]);
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    render(<LogicalCodebaseRegistrationWizard projectId="project_0001" onCompleted={vi.fn()} onClose={vi.fn()} />);

    await user.type(screen.getByLabelText("聚合根目录"), "/root");
    await user.click(screen.getByRole("button", { name: "确认聚合根并自动发现" }));
    expect(await screen.findByText("未发现候选成员，可改为手工预检")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "切换手工预检" }));
    await user.type(screen.getByLabelText("候选成员路径"), "/root/manual");
    await user.click(screen.getByRole("button", { name: "执行手工预检" }));

    const preflightCalls = fetchMock.mock.calls.filter(([input]) => String(input).endsWith("/preflight"));
    expect(preflightCalls).toHaveLength(2);
    expect(JSON.parse(String(preflightCalls[1]?.[1]?.body))).toEqual({
      aggregate_root: "/root",
      candidate_paths: ["/root/manual"],
      auto_discover: false,
    });
  });
});
