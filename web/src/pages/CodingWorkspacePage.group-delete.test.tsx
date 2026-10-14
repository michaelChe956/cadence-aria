import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi, type Mock } from "vitest";
import type { CodingAttemptAddress, CodingExecutionUnit } from "../api/types";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import {
  CODING_ATTEMPT_ADDRESS,
  deferred,
  installCodingWorkspacePageTestHooks,
  mockCodingWs,
} from "./CodingWorkspacePage.test-utils";

vi.mock("../hooks/useCodingWorkspaceWs", () => ({
  useCodingWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
}));

vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => <div>{value}</div>,
}));

vi.mock("../components/shared/MonacoDiffViewer", () => ({
  MonacoDiffViewer: ({ modified }: { modified: string }) => <div>{modified}</div>,
}));

// H5（设计 C/D）：plan 整组删除与单删防呆契约——真实 client（URL/ApiRequestError
// 不 mock，instanceof 生效），fetch 路由模拟 GET /coding-start（含
// workspace_lifecycle 事实）与 DELETE …/coding-workspaces（服务端分类：幂等
// 200 / required 409 / typed 409）。
const CODING_START_URL =
  "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001/coding-start";
const GROUP_DELETE_URL_PREFIX =
  "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001/coding-workspaces";
const ATTEMPT_DELETE_URL =
  "/api/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001";

type FetchRouter = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;
type FetchRouterMock = Mock<FetchRouter>;
type RoutedResponse = { status: number; body: unknown | Promise<unknown> };

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function planProgressRecord(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    plan_id: "issue_plan_0001",
    bound_plan_revision_id: "plan_revision_0002",
    command_id: "plan_start_abc123",
    mode: "first_ready_unit",
    phase: "started",
    primary_attempt_id: "coding_attempt_0001",
    selected_unit_id: "coding_unit_0001",
    selected_attempt_id: "coding_attempt_0001",
    target_attempts: [
      {
        target_repository_id: "repo_busi",
        attempt_id: "coding_attempt_0001",
        attempt_status: "running",
        worktree_path: "/tmp/wt/busi",
        lease_state: "held",
      },
      {
        target_repository_id: "repo_web",
        attempt_id: "coding_attempt_web_0001",
        attempt_status: "created",
        worktree_path: null,
        lease_state: "absent",
      },
      {
        target_repository_id: "repo_gateway",
        attempt_id: "coding_attempt_gateway_0001",
        attempt_status: "created",
        worktree_path: null,
        lease_state: "absent",
      },
      {
        target_repository_id: "repo_frontend",
        attempt_id: "coding_attempt_frontend_0001",
        attempt_status: "created",
        worktree_path: null,
        lease_state: "absent",
      },
    ],
    units: [],
    plan_complete: false,
    workspace_lifecycle: {
      generation: 7,
      existing_member_count: 4,
      missing_member_count: 0,
      state: "clean",
    },
    ...overrides,
  };
}

function deletionResultRecord(): Record<string, unknown> {
  return {
    plan_id: "issue_plan_0001",
    command_id: "placeholder",
    operation_state: "accepted",
    generation: 7,
    deleted_attempt_ids: [
      "coding_attempt_0001",
      "coding_attempt_web_0001",
      "coding_attempt_gateway_0001",
      "coding_attempt_frontend_0001",
    ],
    remaining_attempt_ids: [],
  };
}

const REQUIRED_409: RoutedResponse = {
  status: 409,
  body: {
    code: "deletion_confirmation_required",
    message:
      "将终止运行并删除本计划全部 4 个 Coding 工作区的日志、测试输出与 worktree，计划与工作项保留，且不可恢复。",
    details: { plan_id: "issue_plan_0001" },
  },
};

function groupPlanFetchRouter(options: {
  progress?: Record<string, unknown>;
  deletes?: RoutedResponse[];
}): FetchRouterMock {
  const deletes = [...(options.deletes ?? [])];
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url === CODING_START_URL) {
      return jsonResponse(options.progress ?? planProgressRecord());
    }
    if (url.startsWith(GROUP_DELETE_URL_PREFIX) && init?.method === "DELETE") {
      const next = deletes.shift() ?? { status: 200, body: deletionResultRecord() };
      const body = next.body instanceof Promise ? await next.body : next.body;
      return jsonResponse(body, next.status);
    }
    if (url === ATTEMPT_DELETE_URL && init?.method === "DELETE") {
      return jsonResponse({ status: "deleted" });
    }
    return jsonResponse({ code: "not_found", message: "no route" }, 404);
  });
}

function groupUnit(index: number): CodingExecutionUnit {
  return {
    unit_id: `coding_unit_000${index}`,
    logical_work_item_id: `work_item_000${index}`,
    work_item_revision_id: `work_item_revision_000${index}`,
    dependency_logical_work_item_ids: [],
    order_index: index - 1,
    status: "pending",
    summary: null,
    latest_handoff_revision_id: null,
    completion_commit: null,
  };
}

const GROUP_UNITS = [groupUnit(1), groupUnit(2), groupUnit(3), groupUnit(4)];

function setGroupReadyState(units: CodingExecutionUnit[] = GROUP_UNITS) {
  useCodingWorkspaceStore.setState({
    projectId: "project_0001",
    issueId: "issue_0001",
    attemptId: "coding_attempt_0001",
    status: "running",
    stage: "coding",
    attemptScope: "work_item_group",
    workItemGroupId: "issue_plan_0001",
    units,
  });
}

function groupDeleteUrls(fetchMock: FetchRouterMock): string[] {
  return fetchMock.mock.calls
    .filter(
      ([input, init]) =>
        String(input).startsWith(GROUP_DELETE_URL_PREFIX) && init?.method === "DELETE",
    )
    .map(([input]) => String(input));
}

function attemptDeleteCount(fetchMock: FetchRouterMock): number {
  return fetchMock.mock.calls.filter(
    ([input, init]) => String(input) === ATTEMPT_DELETE_URL && init?.method === "DELETE",
  ).length;
}

describe("CodingWorkspacePage H5 plan-scoped group delete", () => {
  installCodingWorkspacePageTestHooks();

  it("aggregates one plan-scoped delete: idempotent success performs zero confirm dialogs", async () => {
    mockCodingWs();
    const fetchMock = groupPlanFetchRouter({});
    vi.stubGlobal("fetch", fetchMock);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    setGroupReadyState();
    const onBack = vi.fn();

    try {
      render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBack} />);
      // 聚合分支就绪标记先等 GET 落地（入口在局部/slim 分支也立即可见）。
      await screen.findByTestId("plan-coding-progress");
      const button = screen.getByTestId("plan-coding-delete-all");
      expect(button).toHaveTextContent("删除本计划全部 Coding 工作区(4)");
      await userEvent.click(button);

      await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
      const urls = groupDeleteUrls(fetchMock);
      expect(urls).toHaveLength(1);
      const query = new URL(urls[0] ?? "", "http://aria.test");
      expect(query.searchParams.get("expected_generation")).toBe("7");
      expect(query.searchParams.get("command_id")).toMatch(/^plan_delete_ui_/);
      expect(urls[0]).not.toContain("confirmed=true");
      expect(confirm).not.toHaveBeenCalled();
    } finally {
      confirm.mockRestore();
      vi.unstubAllGlobals();
    }
  });

  it("server classified required 409: exactly one confirm; cancel keeps zero destructive follow-ups", async () => {
    mockCodingWs();
    const fetchMock = groupPlanFetchRouter({
      deletes: [REQUIRED_409, { status: 200, body: deletionResultRecord() }],
    });
    vi.stubGlobal("fetch", fetchMock);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    setGroupReadyState();
    const onBack = vi.fn();

    try {
      render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBack} />);
      await screen.findByTestId("plan-coding-progress");
      await userEvent.click(screen.getByTestId("plan-coding-delete-all"));

      await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
      // 消费服务端分类文案（浅/强分级由服务端权威给出）。
      expect(confirm.mock.calls[0]?.[0]).toContain("终止");
      await waitFor(() =>
        expect(screen.getByTestId("plan-coding-delete-all")).toBeEnabled(),
      );
      expect(groupDeleteUrls(fetchMock)).toHaveLength(1);
      expect(onBack).not.toHaveBeenCalled();
      expect(screen.getByTestId("coding-status-bar")).not.toHaveTextContent("失败");
    } finally {
      confirm.mockRestore();
      vi.unstubAllGlobals();
    }
  });

  it("confirmed retry re-issues the same command_id and frozen generation with confirmed=true", async () => {
    mockCodingWs();
    const fetchMock = groupPlanFetchRouter({
      deletes: [REQUIRED_409, { status: 200, body: deletionResultRecord() }],
    });
    vi.stubGlobal("fetch", fetchMock);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    setGroupReadyState();
    const onBack = vi.fn();

    try {
      render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBack} />);
      await screen.findByTestId("plan-coding-progress");
      await userEvent.click(screen.getByTestId("plan-coding-delete-all"));

      await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
      const urls = groupDeleteUrls(fetchMock);
      expect(urls).toHaveLength(2);
      const first = new URL(urls[0] ?? "", "http://aria.test");
      const second = new URL(urls[1] ?? "", "http://aria.test");
      expect(first.searchParams.get("confirmed")).toBeNull();
      expect(second.searchParams.get("confirmed")).toBe("true");
      expect(second.searchParams.get("command_id")).toBe(first.searchParams.get("command_id"));
      expect(second.searchParams.get("expected_generation")).toBe("7");
      expect(confirm).toHaveBeenCalledTimes(1);
    } finally {
      confirm.mockRestore();
      vi.unstubAllGlobals();
    }
  });

  it("typed 409 (resource busy/clearing) shows the server message without a confirm dialog", async () => {
    mockCodingWs();
    const fetchMock = groupPlanFetchRouter({
      deletes: [
        {
          status: 409,
          body: {
            code: "plan_coding_run_resource_busy",
            message: "目标仓资源正被使用，本次未完成。请稍后重试。",
            details: { plan_id: "issue_plan_0001" },
          },
        },
      ],
    });
    vi.stubGlobal("fetch", fetchMock);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    setGroupReadyState();
    const onBack = vi.fn();

    try {
      render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBack} />);
      await screen.findByTestId("plan-coding-progress");
      await userEvent.click(screen.getByTestId("plan-coding-delete-all"));

      await waitFor(() =>
        expect(screen.getByTestId("coding-status-bar")).toHaveTextContent(
          "目标仓资源正被使用，本次未完成。请稍后重试。",
        ),
      );
      expect(confirm).not.toHaveBeenCalled();
      expect(groupDeleteUrls(fetchMock)).toHaveLength(1);
      expect(onBack).not.toHaveBeenCalled();
      await waitFor(() =>
        expect(screen.getByTestId("plan-coding-delete-all")).toBeEnabled(),
      );
    } finally {
      confirm.mockRestore();
      vi.unstubAllGlobals();
    }
  });

  it("omits expected_generation when the aggregate carries no workspace_lifecycle facts", async () => {
    mockCodingWs();
    const progress = planProgressRecord();
    delete progress.workspace_lifecycle;
    const fetchMock = groupPlanFetchRouter({ progress });
    vi.stubGlobal("fetch", fetchMock);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    setGroupReadyState();
    const onBack = vi.fn();

    try {
      render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBack} />);
      await screen.findByTestId("plan-coding-progress");
      await userEvent.click(screen.getByTestId("plan-coding-delete-all"));

      await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
      const urls = groupDeleteUrls(fetchMock);
      expect(urls).toHaveLength(1);
      const query = new URL(urls[0] ?? "", "http://aria.test");
      expect(query.searchParams.has("expected_generation")).toBe(false);
      expect(query.searchParams.get("command_id")).toMatch(/^plan_delete_ui_/);
    } finally {
      confirm.mockRestore();
      vi.unstubAllGlobals();
    }
  });

  it("ignores a stale group delete result after switching the full address", async () => {
    mockCodingWs();
    const pending = deferred<void>();
    const fetchMock = groupPlanFetchRouter({
      deletes: [{ status: 200, body: pending.promise.then(() => deletionResultRecord()) }],
    });
    vi.stubGlobal("fetch", fetchMock);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    setGroupReadyState();
    const onBackA = vi.fn();
    const onBackB = vi.fn();
    const addressB: CodingAttemptAddress = {
      projectId: "project_0002",
      issueId: "issue_0002",
      attemptId: "coding_attempt_0002",
    };

    try {
      const view = render(
        <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBackA} />,
      );
      await screen.findByTestId("plan-coding-progress");
      await userEvent.click(screen.getByTestId("plan-coding-delete-all"));

      useCodingWorkspaceStore.setState({
        projectId: addressB.projectId,
        issueId: addressB.issueId,
        attemptId: addressB.attemptId,
        workItemGroupId: null,
        attemptScope: "work_item",
      });
      view.rerender(<CodingWorkspacePage address={addressB} onBack={onBackB} />);

      await act(async () => {
        pending.resolve(undefined);
        await pending.promise.catch(() => undefined);
      });

      expect(onBackA).not.toHaveBeenCalled();
      expect(onBackB).not.toHaveBeenCalled();
    } finally {
      confirm.mockRestore();
      vi.unstubAllGlobals();
    }
  });

  it("disables the entry while a group delete request is in flight", async () => {
    mockCodingWs();
    const pending = deferred<void>();
    const fetchMock = groupPlanFetchRouter({
      deletes: [{ status: 200, body: pending.promise.then(() => deletionResultRecord()) }],
    });
    vi.stubGlobal("fetch", fetchMock);
    setGroupReadyState();
    const onBack = vi.fn();

    try {
      render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBack} />);
      await screen.findByTestId("plan-coding-progress");
      await userEvent.click(screen.getByTestId("plan-coding-delete-all"));
      expect(screen.getByTestId("plan-coding-delete-all")).toBeDisabled();

      await act(async () => {
        pending.resolve(undefined);
        await pending.promise.catch(() => undefined);
      });
      expect(groupDeleteUrls(fetchMock)).toHaveLength(1);
      await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("keeps the recovery entry reachable when group units are empty but the plan id is confirmed", async () => {
    mockCodingWs();
    const fetchMock = groupPlanFetchRouter({});
    vi.stubGlobal("fetch", fetchMock);
    setGroupReadyState([]);
    const onBack = vi.fn();

    try {
      render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBack} />);
      // slim 行立即出现（恢复可达）；计数在聚合就绪后补全（分支切换需重取按钮）。
      await screen.findByTestId("plan-coding-delete-all");
      await screen.findByTestId("plan-coding-progress");
      const button = screen.getByTestId("plan-coding-delete-all");
      expect(button).toHaveTextContent("删除本计划全部 Coding 工作区(4)");
      await userEvent.click(button);
      await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe("CodingWorkspacePage H5 single attempt delete copy (design D)", () => {
  installCodingWorkspacePageTestHooks();

  function setSingleState(groupScope: boolean) {
    useCodingWorkspaceStore.setState({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
      attemptScope: groupScope ? "work_item_group" : "work_item",
      workItemGroupId: groupScope ? "issue_plan_0001" : null,
      units: groupScope ? GROUP_UNITS : [],
    });
  }

  it("labels the top bar entry as 仅删除当前 Coding 工作区 and keeps v1 single-scope copy", async () => {
    mockCodingWs();
    const fetchMock = groupPlanFetchRouter({});
    vi.stubGlobal("fetch", fetchMock);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    setSingleState(false);
    const onBack = vi.fn();

    try {
      render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBack} />);
      await userEvent.click(
        screen.getByRole("button", { name: "仅删除当前 Coding 工作区" }),
      );

      expect(confirm).toHaveBeenCalledTimes(1);
      expect(confirm.mock.calls[0]?.[0]).toContain("日志、测试输出和 worktree");
      expect(confirm.mock.calls[0]?.[0]).not.toContain("整轮授权失效");
      await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
      expect(attemptDeleteCount(fetchMock)).toBe(1);
    } finally {
      confirm.mockRestore();
      vi.unstubAllGlobals();
    }
  });

  it("group scope single delete warns the round is invalidated and points to the group entry", async () => {
    mockCodingWs();
    const fetchMock = groupPlanFetchRouter({});
    vi.stubGlobal("fetch", fetchMock);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    setSingleState(true);
    const onBack = vi.fn();

    try {
      render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBack} />);
      await userEvent.click(
        screen.getByRole("button", { name: "仅删除当前 Coding 工作区" }),
      );

      expect(confirm).toHaveBeenCalledTimes(1);
      // P2-2：units.length 是工作项数，冒充工作区成员数会 N×R 失真——恒用泛化句。
      expect(confirm.mock.calls[0]?.[0]).toContain("这是本计划的 Coding 工作区之一");
      expect(confirm.mock.calls[0]?.[0]).not.toMatch(/这是本计划 \d+ 个/u);
      expect(confirm.mock.calls[0]?.[0]).toContain("整轮授权失效");
      expect(confirm.mock.calls[0]?.[0]).toContain("删除本计划全部 Coding 工作区");
      expect(onBack).not.toHaveBeenCalled();
      expect(attemptDeleteCount(fetchMock)).toBe(0);
    } finally {
      confirm.mockRestore();
      vi.unstubAllGlobals();
    }
  });
});

describe("CodingWorkspacePage H5 P2 修复轮:双删除 busy 互斥与地址复位", () => {
  installCodingWorkspacePageTestHooks();

  const ADDRESS_B: CodingAttemptAddress = {
    projectId: "project_0002",
    issueId: "issue_0002",
    attemptId: "coding_attempt_0002",
  };

  it("单删在途时整组入口互斥:零组删出站,单删结算后顶栏 busy 复位不卡死", async () => {
    mockCodingWs();
    const pending = deferred<void>();
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url === CODING_START_URL) {
        return jsonResponse(planProgressRecord());
      }
      if (url === ATTEMPT_DELETE_URL && init?.method === "DELETE") {
        await pending.promise;
        return jsonResponse({ status: "deleted" });
      }
      if (url.startsWith(GROUP_DELETE_URL_PREFIX) && init?.method === "DELETE") {
        return jsonResponse(deletionResultRecord());
      }
      return jsonResponse({ code: "not_found", message: "no route" }, 404);
    });
    vi.stubGlobal("fetch", fetchMock);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    setGroupReadyState();
    const onBack = vi.fn();

    try {
      render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBack} />);
      await screen.findByTestId("plan-coding-progress");
      const topBar = screen.getByRole("button", { name: "仅删除当前 Coding 工作区" });
      const groupEntry = screen.getByTestId("plan-coding-delete-all");

      await userEvent.click(topBar);
      expect(topBar).toBeDisabled();
      // P3：互斥语义反映到控件可见态——单删在途时组删入口同步禁用。
      expect(groupEntry).toBeDisabled();

      // 互斥：单删在途时整组入口零出站（现实现会顶号发起组删）。
      await userEvent.click(groupEntry);
      // 给在途异步链一个结算窗口（若互斥失效，组删会在此窗口内出站）。
      const settle = deferred<void>();
      setTimeout(settle.resolve, 50);
      await act(async () => {
        await settle.promise;
      });
      expect(groupDeleteUrls(fetchMock)).toHaveLength(0);

      await act(async () => {
        pending.resolve(undefined);
        await pending.promise.catch(() => undefined);
      });
      await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
      // busy 复位：顶栏不因组删顶号而永久禁用。
      await waitFor(() =>
        expect(
          screen.getByRole("button", { name: "仅删除当前 Coding 工作区" }),
        ).toBeEnabled(),
      );
      expect(attemptDeleteCount(fetchMock)).toBe(1);
    } finally {
      confirm.mockRestore();
      vi.unstubAllGlobals();
    }
  });

  it("整组在途时单删入口互斥:不弹确认零出站,组删结算后单删恢复可用", async () => {
    mockCodingWs();
    const pending = deferred<void>();
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url === CODING_START_URL) {
        return jsonResponse(planProgressRecord());
      }
      if (url.startsWith(GROUP_DELETE_URL_PREFIX) && init?.method === "DELETE") {
        await pending.promise;
        return jsonResponse(deletionResultRecord());
      }
      if (url === ATTEMPT_DELETE_URL && init?.method === "DELETE") {
        return jsonResponse({ status: "deleted" });
      }
      return jsonResponse({ code: "not_found", message: "no route" }, 404);
    });
    vi.stubGlobal("fetch", fetchMock);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    setGroupReadyState();
    const onBack = vi.fn();

    try {
      render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBack} />);
      await screen.findByTestId("plan-coding-progress");
      await userEvent.click(screen.getByTestId("plan-coding-delete-all"));
      expect(screen.getByTestId("plan-coding-delete-all")).toBeDisabled();
      // P3：互斥语义反映到控件可见态——组删在途时单删入口同步禁用。
      expect(
        screen.getByRole("button", { name: "仅删除当前 Coding 工作区" }),
      ).toBeDisabled();

      // 互斥：整组在途时单删入口直接返回（不弹确认、零出站）。
      await userEvent.click(
        screen.getByRole("button", { name: "仅删除当前 Coding 工作区" }),
      );
      expect(confirm).not.toHaveBeenCalled();
      expect(attemptDeleteCount(fetchMock)).toBe(0);

      await act(async () => {
        pending.resolve(undefined);
        await pending.promise.catch(() => undefined);
      });
      await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
      // 组删结算后单删入口恢复可用（busy 复位）。
      await waitFor(() =>
        expect(
          screen.getByRole("button", { name: "仅删除当前 Coding 工作区" }),
        ).toBeEnabled(),
      );
    } finally {
      confirm.mockRestore();
      vi.unstubAllGlobals();
    }
  });

  it("地址切换复位整组删除 busy,回到组页后入口不卡死且可再发起", async () => {
    mockCodingWs();
    const pending = deferred<void>();
    let groupDeleteCalls = 0;
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url === CODING_START_URL) {
        return jsonResponse(planProgressRecord());
      }
      if (url.startsWith(GROUP_DELETE_URL_PREFIX) && init?.method === "DELETE") {
        groupDeleteCalls += 1;
        if (groupDeleteCalls === 1) {
          await pending.promise;
        }
        return jsonResponse(deletionResultRecord());
      }
      return jsonResponse({ code: "not_found", message: "no route" }, 404);
    });
    vi.stubGlobal("fetch", fetchMock);
    setGroupReadyState();
    const onBackA = vi.fn();
    const onBackB = vi.fn();

    try {
      const view = render(
        <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBackA} />,
      );
      await screen.findByTestId("plan-coding-progress");
      await userEvent.click(screen.getByTestId("plan-coding-delete-all"));
      expect(screen.getByTestId("plan-coding-delete-all")).toBeDisabled();

      // 地址切换：effect 复位 groupDeleteBusy（现实现只复位 deleteBusy）。
      useCodingWorkspaceStore.setState({
        projectId: ADDRESS_B.projectId,
        issueId: ADDRESS_B.issueId,
        attemptId: ADDRESS_B.attemptId,
        workItemGroupId: null,
        attemptScope: "work_item",
      });
      view.rerender(<CodingWorkspacePage address={ADDRESS_B} onBack={vi.fn()} />);
      await act(async () => {
        pending.resolve(undefined);
        await pending.promise.catch(() => undefined);
      });
      expect(onBackA).not.toHaveBeenCalled();

      // 回到组页：入口可用（不卡死），可再次发起组删。
      setGroupReadyState();
      view.rerender(
        <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBackB} />,
      );
      const reEntry = await screen.findByTestId("plan-coding-delete-all");
      await waitFor(() => expect(reEntry).toBeEnabled());
      // slim→聚合分支切换会替换节点，点击前重取挂载中的引用。
      await userEvent.click(screen.getByTestId("plan-coding-delete-all"));
      await waitFor(() => expect(onBackB).toHaveBeenCalledTimes(1));
      expect(groupDeleteCalls).toBe(2);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
