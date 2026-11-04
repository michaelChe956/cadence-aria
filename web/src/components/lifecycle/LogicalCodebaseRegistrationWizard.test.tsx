import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LogicalCodebaseRegistrationWizard } from "./LogicalCodebaseRegistrationWizard";

/** 七个布局码的对外稳定码（块2 码表为键；块3 只消费 `details.copy`）。 */
const LAYOUT_CODES = [
  "aggregate_root_is_git",
  "aggregate_root_member_outside_root",
  "aggregate_root_member_symlink_escape",
  "aggregate_root_nested_worktree",
  "aggregate_root_overlap",
  "aggregate_index_layout_unsupported",
  "aggregate_index_member_coverage_failed",
] as const;

/** 后端 `FailureCopy` 的同构夹具：文案内容由码派生（改写分隔符，避免与
 *  机器码字面重复），测试断言只做往返比对。 */
function layoutCopy(code: string) {
  const slug = code.split("_").join("·");
  return {
    title_zh: `问题 ${slug}`,
    reason_zh: `原因 ${slug}`,
    action_zh: `怎么做 ${slug}`,
    triage: "environment" as const,
    machine_line: code,
  };
}

function installTarget(
  overrides: Partial<{
    target: string;
    display_name: string;
    cli_available: boolean;
    local_version: string | null;
    latest_version: string | null;
    latest_checked_at: string | null;
    install_source: string;
    action: string;
  }> = {},
) {
  return {
    target: "codegraph",
    display_name: "codegraph",
    cli_available: false,
    local_version: null,
    latest_version: "1.7.0",
    latest_checked_at: null,
    install_source: "npm",
    action: "install",
    ...overrides,
  };
}

/** 预检失败（带 copy）+ 安装目标面：用于 codegraph 前置的入口引导用例。 */
function codegraphErrorFetch(code: string, targets: unknown[]) {
  return vi.fn(async (input: RequestInfo | URL, _init?: RequestInit) => {
    const path = String(input);
    if (path.endsWith("/api/providers/install-targets")) {
      return jsonResponse({ targets });
    }
    return jsonResponse(
      {
        code,
        message: "aggregate root preflight rejected",
        details: { copy: layoutCopy(code) },
      },
      422,
    );
  });
}

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

async function manualPreflightAlert(payload: unknown): Promise<HTMLElement> {
  vi.stubGlobal("fetch", vi.fn(async () => jsonResponse(payload, 422)));
  const user = userEvent.setup();
  render(
    <LogicalCodebaseRegistrationWizard
      projectId="project_0001"
      logicalCodebaseId="lc_0001"
      onCompleted={vi.fn()}
      onClose={vi.fn()}
    />,
  );
  await user.click(screen.getByRole("button", { name: "改为手工预检" }));
  await user.type(screen.getByLabelText("聚合根目录"), "/x");
  await user.type(screen.getByLabelText("候选成员路径"), "/x/api");
  await user.click(screen.getByRole("button", { name: "执行手工预检" }));
  return screen.findByRole("alert");
}

describe("LogicalCodebaseRegistrationWizard", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("自动拉取候选、按分类初始化勾选，并只提交勾选项", async () => {
    const fetchMock = registrationFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    const onCompleted = vi.fn();
    render(<LogicalCodebaseRegistrationWizard projectId="project_0001" logicalCodebaseId="lc_0001" onCompleted={onCompleted} onClose={vi.fn()} />);

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
      String(input).endsWith("/logical-codebases/lc_0001/registrations"),
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
    render(<LogicalCodebaseRegistrationWizard projectId="project_0001" logicalCodebaseId="lc_0001" onCompleted={vi.fn()} onClose={vi.fn()} />);

    await user.type(screen.getByLabelText("聚合根目录"), "/root");
    await user.click(screen.getByRole("button", { name: "确认聚合根并自动发现" }));
    await user.click(screen.getByRole("button", { name: "提交登记" }));

    const submitCall = fetchMock.mock.calls.find(([input]) =>
      String(input).endsWith("/logical-codebases/lc_0001/registrations"),
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
    render(<LogicalCodebaseRegistrationWizard projectId="project_0001" logicalCodebaseId="lc_0001" onCompleted={vi.fn()} onClose={vi.fn()} />);

    await user.type(screen.getByLabelText("聚合根目录"), "/missing");
    await user.click(screen.getByRole("button", { name: "确认聚合根并自动发现" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("aggregate_root_missing: 聚合根不存在");
    expect(screen.getByLabelText("候选成员路径")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "执行手工预检" })).toBeInTheDocument();
  });

  it("自动发现为空时保留手工预检兜底", async () => {
    const fetchMock = registrationFetch([]);
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    render(<LogicalCodebaseRegistrationWizard projectId="project_0001" logicalCodebaseId="lc_0001" onCompleted={vi.fn()} onClose={vi.fn()} />);

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

  it("embedded：预填聚合根并自动触发自动发现，不渲染对话框壳", async () => {
    const fetchMock = registrationFetch();
    vi.stubGlobal("fetch", fetchMock);

    render(
      <LogicalCodebaseRegistrationWizard
        projectId="project_0001"
        logicalCodebaseId="lc_0001"
        onCompleted={vi.fn()}
        embedded
        initialAggregateRoot="/root/mono"
        autoStart
      />,
    );

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByLabelText("聚合根目录")).toHaveValue("/root/mono");

    const preflightCall = fetchMock.mock.calls.find(([input]) =>
      String(input).endsWith("/preflight"),
    );
    expect(JSON.parse(String(preflightCall?.[1]?.body))).toEqual({
      aggregate_root: "/root/mono",
      candidate_paths: [],
      auto_discover: true,
    });

    expect(
      await screen.findByLabelText("选择 /root/api（eligible）"),
    ).toBeInTheDocument();
  });

  it("embedded：自动发现结果旁保留改为手工预检入口", async () => {
    const fetchMock = registrationFetch();
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(
      <LogicalCodebaseRegistrationWizard
        projectId="project_0001"
        logicalCodebaseId="lc_0001"
        onCompleted={vi.fn()}
        embedded
        initialAggregateRoot="/root"
        autoStart
      />,
    );

    await screen.findByLabelText("选择 /root/api（eligible）");
    await user.click(screen.getByRole("button", { name: "改为手工预检" }));
    expect(screen.getByLabelText("候选成员路径")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "执行手工预检" })).toBeInTheDocument();
    expect(
      screen.queryByLabelText("选择 /root/api（eligible）"),
    ).not.toBeInTheDocument();
  });

  it("手工预检失败时优先展示 details.reason（ownership_conflict 样例）", async () => {
    const alert = await manualPreflightAlert({
      code: "aggregate_root_ownership_conflict",
      message: "aggregate root preflight rejected",
      details: {
        reason:
          "aggregate root /x already contains user-owned /x/CLAUDE.md; move or merge it before aggregate initialization",
      },
    });
    expect(alert).toHaveTextContent(
      "预检失败：aggregate root /x already contains user-owned /x/CLAUDE.md; move or merge it before aggregate initialization",
    );
    expect(alert).not.toHaveTextContent("aggregate root preflight rejected");
  });

  it("手工预检失败时优先展示 details.reason（aggregate_root_missing 样例）", async () => {
    const alert = await manualPreflightAlert({
      code: "aggregate_root_missing",
      message: "aggregate root preflight rejected",
      details: {
        reason: "could not inspect aggregate root /x: permission denied (os error 13)",
      },
    });
    expect(alert).toHaveTextContent(
      "预检失败：could not inspect aggregate root /x: permission denied (os error 13)",
    );
  });

  it("details.reason 缺失时回落到 code 与 message", async () => {
    const alert = await manualPreflightAlert({
      code: "aggregate_root_missing",
      message: "聚合根不存在",
    });
    expect(alert).toHaveTextContent("aggregate_root_missing: 聚合根不存在");
  });

  it("details.reason 与 message 均缺失时使用兜底文案", async () => {
    const alert = await manualPreflightAlert({
      code: "aggregate_root_missing",
      details: {},
    });
    expect(alert).toHaveTextContent("预检失败");
  });

  // ---- 块3 片5：目录树 + 三段式失败面 + 安装入口 ----

  it("entry_shows_directory_tree_diagram", async () => {
    vi.stubGlobal("fetch", registrationFetch());
    render(
      <LogicalCodebaseRegistrationWizard
        projectId="project_0001"
        logicalCodebaseId="lc_0001"
        onClose={vi.fn()}
      />,
    );

    // REQ-LAYOUT-02：登记入口必须展示目录树示意。
    expect(screen.getByTestId("lc-layout-tree")).toBeInTheDocument();
    expect(screen.getByTestId("lc-layout-tree-root")).toHaveTextContent(
      "my-lc-root/",
    );
  });

  it("format_error_prefers_details_copy", async () => {
    const copy = layoutCopy("aggregate_root_is_git");
    const alert = await manualPreflightAlert({
      code: "aggregate_root_is_git",
      message: "aggregate root preflight rejected",
      details: { reason: "（原始 reason 不应顶替成品）", copy },
    });

    // 三段分行：问题 / 原因 / 怎么做各自独立元素。
    expect(
      within(alert).getByTestId("lc-registration-error-title"),
    ).toHaveTextContent(copy.title_zh);
    expect(
      within(alert).getByTestId("lc-registration-error-reason"),
    ).toHaveTextContent(copy.reason_zh);
    expect(
      within(alert).getByTestId("lc-registration-error-action"),
    ).toHaveTextContent(copy.action_zh);

    // 机器码降到次要行：独立元素、等宽小字、位于最后一行，且不进主行。
    const machineLine = within(alert).getByTestId(
      "lc-registration-error-machine-line",
    );
    expect(machineLine).toHaveTextContent(copy.machine_line);
    expect(machineLine.className).toContain("font-mono");
    expect(alert.lastElementChild).toBe(machineLine);
    expect(
      within(alert).getByTestId("lc-registration-error-title"),
    ).not.toHaveTextContent(copy.machine_line);

    // copy 优先于 details.reason。
    expect(alert).not.toHaveTextContent("原始 reason 不应顶替成品");
  });

  it("format_error_falls_back_when_copy_missing", async () => {
    const alert = await manualPreflightAlert({
      code: "aggregate_root_missing",
      message: "aggregate root preflight rejected",
      details: { reason: "could not inspect aggregate root /x" },
    });

    // 现行为不变：details.reason 优先，单行呈现，不出现三段/机器码次要行。
    expect(alert).toHaveTextContent(
      "预检失败：could not inspect aggregate root /x",
    );
    expect(
      within(alert).queryByTestId("lc-registration-error-action"),
    ).toBeNull();
    expect(
      within(alert).queryByTestId("lc-registration-error-machine-line"),
    ).toBeNull();
  });

  it.each(LAYOUT_CODES)(
    "each_layout_error_has_action_text：%s",
    async (code) => {
      const copy = layoutCopy(code);
      const alert = await manualPreflightAlert({
        code,
        message: "aggregate root preflight rejected",
        details: { copy },
      });

      // 七个布局码逐条：三段齐全（含 action），机器码在次要行。
      expect(
        within(alert).getByTestId("lc-registration-error-title"),
      ).toHaveTextContent(copy.title_zh);
      expect(
        within(alert).getByTestId("lc-registration-error-reason"),
      ).toHaveTextContent(copy.reason_zh);
      expect(
        within(alert).getByTestId("lc-registration-error-action"),
      ).toHaveTextContent(copy.action_zh);
      expect(
        within(alert).getByTestId("lc-registration-error-machine-line"),
      ).toHaveTextContent(copy.machine_line);
    },
  );

  it("codegraph_missing_shows_install_entry", async () => {
    vi.stubGlobal(
      "fetch",
      codegraphErrorFetch("codegraph_missing", [
        installTarget({ cli_available: false }),
      ]),
    );
    const user = userEvent.setup();
    render(
      <LogicalCodebaseRegistrationWizard
        projectId="project_0001"
        logicalCodebaseId="lc_0001"
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("聚合根目录"), "/x");
    await user.click(screen.getByRole("button", { name: "确认聚合根并自动发现" }));

    // H3：安装一律走块2 的入口（含手动命令兜底）。
    expect(
      await screen.findByTestId("provider-install-start-codegraph"),
    ).toHaveTextContent("安装");
    expect(
      screen.getByTestId("provider-install-manual-command-codegraph"),
    ).toHaveTextContent("npm install -g");
  });

  it("codegraph_version_mismatch_shows_update_entry", async () => {
    vi.stubGlobal(
      "fetch",
      codegraphErrorFetch("codegraph_version_mismatch", [
        installTarget({
          cli_available: true,
          local_version: "1.6.0",
          latest_version: "1.7.0",
          action: "update",
        }),
      ]),
    );
    const user = userEvent.setup();
    render(
      <LogicalCodebaseRegistrationWizard
        projectId="project_0001"
        logicalCodebaseId="lc_0001"
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("聚合根目录"), "/x");
    await user.click(screen.getByRole("button", { name: "确认聚合根并自动发现" }));

    expect(
      await screen.findByTestId("provider-install-start-codegraph"),
    ).toHaveTextContent("更新");
    expect(
      screen.getByTestId("provider-install-versions-codegraph"),
    ).toHaveTextContent("当前 1.6.0 → 最新 1.7.0");
    // 向导场景 capabilityConfirmed=true：不得渲染（禁用的）【核验】。
    expect(
      screen.queryByTestId("provider-install-verify-codegraph"),
    ).toBeNull();
  });
});
