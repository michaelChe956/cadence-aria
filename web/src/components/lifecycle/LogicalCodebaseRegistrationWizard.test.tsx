import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LogicalCodebaseRegistrationWizard } from "./LogicalCodebaseRegistrationWizard";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function registrationFetch() {
  const submit = { batch_id: "batch_0001", status: "partial_failed", items: [
    { path: "/root/api", status: "completed", failure_reason: null },
    { path: "/root/web", status: "failed", failure_reason: "dirty" },
  ] };
  const resume = { ...submit, status: "completed", items: submit.items.map((item) => ({ ...item, status: "completed", failure_reason: null })) };
  return vi.fn(async (input: RequestInfo | URL) => {
    const path = String(input);
    if (path.endsWith("/preflight")) {
      return jsonResponse({ preflight_id: "preflight_0001", created_at: "2026-08-18T00:00:00Z", items: [
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

  it("preflights, explicitly confirms needs_attention, submits, and resumes partial batch", async () => {
    vi.stubGlobal("fetch", registrationFetch());
    const user = userEvent.setup();
    const onCompleted = vi.fn();
    render(<LogicalCodebaseRegistrationWizard projectId="project_0001" onCompleted={onCompleted} onClose={vi.fn()} />);

    await user.type(screen.getByLabelText("聚合根目录"), "/root");
    await user.type(screen.getByLabelText("候选成员路径"), "/root/api\n/root/web");
    await user.click(screen.getByRole("button", { name: "执行预检" }));

    expect(await screen.findByText("needs_attention")).toBeInTheDocument();
    expect(screen.getByText("non_git")).toBeInTheDocument();
    expect(screen.getByText("outside_root")).toBeInTheDocument();
    const dirty = screen.getByLabelText("确认 /root/web（dirty）");
    expect(dirty).not.toBeChecked();
    await user.click(dirty);
    await user.click(screen.getByRole("button", { name: "提交登记" }));

    expect(await screen.findByText("partial_failed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "恢复未完成项" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "恢复未完成项" }));
    expect(await screen.findByText("completed")).toBeInTheDocument();
    expect(onCompleted).toHaveBeenCalledTimes(1);
  });

  it("does not submit needs_attention until explicitly confirmed", async () => {
    const fetchMock = registrationFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    render(<LogicalCodebaseRegistrationWizard projectId="project_0001" onCompleted={vi.fn()} onClose={vi.fn()} />);

    await user.type(screen.getByLabelText("聚合根目录"), "/root");
    await user.type(screen.getByLabelText("候选成员路径"), "/root/web");
    await user.click(screen.getByRole("button", { name: "执行预检" }));
    await user.click(screen.getByRole("button", { name: "提交登记" }));
    expect(screen.getByRole("alert")).toHaveTextContent("请确认需要关注的成员");
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
