// useWorkspaceSessionObservers 拆分测试（P2/P3 目录轮询节律：dirty 限速、
// 空闲退避 60s 档、页面不可见暂停）。
//
// 2026-11-13：useWorkspaceSessionObservers.test.tsx 超出 large_file_guard
// 登记上限（tests/it_core/large_file_guard.rs）⇒ 按主题拆分，本文件承接
// 轮询节律组用例；用例与断言逐字保留（拆分自原文件 1297-1561 行），行为
// 零变化。公共夹具见 useWorkspaceSessionObservers.test-utils.tsx。
import { act, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceSessionSummary } from "../api/types";
import { notifyLifecycleInvalidated } from "../state/lifecycle-workbench-store";
import {
  observerOptions,
  renderObserverHook,
  summary,
} from "./useWorkspaceSessionObservers.test-utils";
import type { WorkspaceSessionObserverOptions } from "./useWorkspaceSessionObservers";

afterEach(() => {
  vi.useRealTimers();
});
// P2/P3（无限轮询修复）：dirty 快路径限速与周期不旁路、空闲退避 60s 档、
// 页面不可见暂停——全部经注入的假定时器/假控制器驱动，不触真实计时器。
describe("catalog polling pacing (P2/P3 runaway fixes)", () => {
  interface ScheduledTimer {
    id: number;
    callback: () => void;
    delayMs: number;
    cancelled: boolean;
    fired: boolean;
  }

  const createManualScheduler = () => {
    const timers: ScheduledTimer[] = [];
    let nextId = 1;
    return {
      timers,
      schedule(callback: () => void, delayMs: number) {
        const timer: ScheduledTimer = {
          id: nextId,
          callback,
          delayMs,
          cancelled: false,
          fired: false,
        };
        nextId += 1;
        timers.push(timer);
        return timer.id;
      },
      cancel(id: number) {
        const timer = timers.find((entry) => entry.id === id);
        if (timer) {
          timer.cancelled = true;
        }
      },
      fire(timer: ScheduledTimer) {
        if (timer.cancelled || timer.fired) {
          return;
        }
        timer.fired = true;
        timer.callback();
      },
      pending() {
        return timers.filter((timer) => !timer.cancelled && !timer.fired);
      },
    };
  };

  const setHidden = (hidden: boolean) => {
    Object.defineProperty(document, "hidden", {
      configurable: true,
      get: () => hidden,
    });
  };

  const stubController = () => ({
    replaceWatchedSessionIds: vi.fn(),
    updateRefreshIntervalMs: vi.fn(),
    refresh: vi.fn(),
    records: () => [],
    dispose: vi.fn(),
  });

  afterEach(() => {
    // jsdom：hidden 是原型 getter，删除测试期间定义的自有遮蔽属性以还原。
    const documentWithOwnHidden = document as { hidden?: boolean };
    delete documentWithOwnHidden.hidden;
  });

  it("paces dirty reruns with a minimum gap and keeps the periodic timer armed (P2)", async () => {
    const scheduler = createManualScheduler();
    let releaseLifecycle: (() => void) | undefined;
    const getIssueLifecycle = vi.fn(
      () =>
        new Promise<{ workspace_sessions: WorkspaceSessionSummary[]; coding_attempts: never[] }>(
          (resolve) => {
            releaseLifecycle = () =>
              resolve({ workspace_sessions: [summary("s1")], coding_attempts: [] });
          },
        ),
    );
    const view = renderObserverHook(
      observerOptions({
        refreshIntervalMs: 15_000,
        getIssueLifecycle,
        scheduleCatalogRefresh: scheduler.schedule,
        cancelCatalogRefresh: scheduler.cancel,
        createController: stubController,
      }),
    );

    // 第 1 轮在途时收到失效唤醒 ⇒ 只标 dirty，不起第二轮。
    await act(async () => {
      notifyLifecycleInvalidated("issue_1");
    });
    await act(async () => {
      releaseLifecycle?.();
    });

    // dirty 补跑不再零延迟：第 2 轮没有立即开始。
    expect(getIssueLifecycle).toHaveBeenCalledTimes(1);
    // 周期定时器没有被 dirty 快路径旁路：15s 档仍在排程中。
    expect(scheduler.pending().some((timer) => timer.delayMs === 15_000)).toBe(true);
    // dirty 补跑至少间隔 1s（DIRTY_RERUN_MIN_DELAY_MS 档）再启动。
    const rerunTimer = scheduler
      .pending()
      .find((timer) => timer.delayMs !== 15_000 && timer.delayMs >= 1_000);
    expect(rerunTimer).toBeDefined();

    // 触发补跑：第 2 轮开始；完成后按原节奏重排下一轮。
    await act(async () => {
      if (rerunTimer) {
        scheduler.fire(rerunTimer);
      }
    });
    expect(getIssueLifecycle).toHaveBeenCalledTimes(2);
    await act(async () => {
      releaseLifecycle?.();
    });
    expect(scheduler.pending().some((timer) => timer.delayMs === 15_000)).toBe(true);

    await act(async () => {
      view.unmount();
    });
  });

  it("backs off to the 60s tier when idle and keeps the user cadence with in-flight facts (P3)", async () => {
    const scheduler = createManualScheduler();
    const pacing = (
      getIssueLifecycle: WorkspaceSessionObserverOptions["getIssueLifecycle"],
      overrides: Partial<WorkspaceSessionObserverOptions> = {},
    ) =>
      observerOptions({
        refreshIntervalMs: 15_000,
        getIssueLifecycle,
        scheduleCatalogRefresh: scheduler.schedule,
        cancelCatalogRefresh: scheduler.cancel,
        createController: stubController,
        ...overrides,
      });

    // 空闲：无会话、无等待项、无 LC 通知 ⇒ 下一轮退到 60s 档。
    const idleLifecycle = vi.fn(async () => ({
      workspace_sessions: [],
      coding_attempts: [],
    }));
    const idleView = renderObserverHook(
      pacing(idleLifecycle, {
        listProjectRepositoryInitializationWaitingItems: async () => [],
      }),
    );
    await waitFor(() =>
      expect(scheduler.pending().map((timer) => timer.delayMs)).toEqual([60_000]),
    );
    expect(idleLifecycle).toHaveBeenCalledTimes(1);
    await act(async () => {
      idleView.unmount();
    });

    // 活跃会话（running）⇒ 维持用户档 15s。
    const activeLifecycle = vi.fn(async () => ({
      workspace_sessions: [summary("s1")],
      coding_attempts: [],
    }));
    const activeView = renderObserverHook(pacing(activeLifecycle));
    await waitFor(() =>
      expect(scheduler.pending().map((timer) => timer.delayMs)).toEqual([15_000]),
    );
    expect(activeLifecycle).toHaveBeenCalledTimes(1);
    await act(async () => {
      activeView.unmount();
    });

    // 待处理事实（project 等待项）同样维持用户档 15s。
    const waitingLifecycle = vi.fn(async () => ({
      workspace_sessions: [],
      coding_attempts: [],
    }));
    const waitingView = renderObserverHook(
      pacing(waitingLifecycle, {
        listProjectRepositoryInitializationWaitingItems: async () => [
          {
            id: "c1:project:project_1:repository_init:op_init_0001",
            kind: "repository_initialization_failed",
            reason: "repository initialization failed",
            completed_steps: [],
            target: null,
            plan_id: null,
            session_id: null,
            attempt_id: null,
            gate_id: null,
            possible_side_effect: null,
            actions: ["resume_repository_initialization"],
            next_phase: "repository_registered",
            action_context: [],
            operation_id: "op_init_0001",
            diagnostics: null,
            project_id: "project_1",
          },
        ],
      }),
    );
    await waitFor(() =>
      expect(scheduler.pending().map((timer) => timer.delayMs)).toEqual([15_000]),
    );
    expect(waitingLifecycle).toHaveBeenCalledTimes(1);
    await act(async () => {
      waitingView.unmount();
    });
  });

  it("pauses polling while the page is hidden and refreshes immediately on return (P3)", async () => {
    const scheduler = createManualScheduler();
    const getIssueLifecycle = vi.fn(async () => ({
      workspace_sessions: [summary("s1")],
      coding_attempts: [],
    }));
    const view = renderObserverHook(
      observerOptions({
        refreshIntervalMs: 15_000,
        getIssueLifecycle,
        scheduleCatalogRefresh: scheduler.schedule,
        cancelCatalogRefresh: scheduler.cancel,
        createController: stubController,
      }),
    );
    await waitFor(() => expect(getIssueLifecycle).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(scheduler.pending().some((timer) => timer.delayMs === 15_000)).toBe(true),
    );

    // 隐藏 ⇒ 暂停：已排的周期定时器被撤销。
    setHidden(true);
    await act(async () => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    expect(scheduler.pending().some((timer) => timer.delayMs === 15_000)).toBe(false);

    // 隐藏期间的唤醒（失效 → 250ms 防抖）到期后不发起请求。
    await act(async () => {
      notifyLifecycleInvalidated("issue_1");
    });
    const wakeTimer = scheduler.pending().find((timer) => timer.delayMs !== 15_000);
    expect(wakeTimer).toBeDefined();
    await act(async () => {
      if (wakeTimer) {
        scheduler.fire(wakeTimer);
      }
    });
    expect(getIssueLifecycle).toHaveBeenCalledTimes(1);

    // 回到可见 ⇒ 立刻补一轮并恢复周期排程。
    setHidden(false);
    await act(async () => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    expect(getIssueLifecycle).toHaveBeenCalledTimes(2);
    await waitFor(() =>
      expect(scheduler.pending().some((timer) => timer.delayMs === 15_000)).toBe(true),
    );

    await act(async () => {
      view.unmount();
    });
  });
});
