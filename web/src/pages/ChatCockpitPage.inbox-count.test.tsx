// F-A 第 3 条回归（owner 2026-10-25）：页头「待处理 N」徽标只数**真需要人决策**的条目。
// `kind="info"`（自动等待中 / 只读旧代绑定 / plan 确认信息……）在抽屉里落在「进度信息」
// 分组且没有动作按钮——把它们算进徽标会把"不点也能过"的东西灌成待办，owner 分不清
// 哪些"不点就过不去"。
import { screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type * as ApiClient from "../api/client";
import type * as WorkspaceWsModule from "../hooks/useWorkspaceWs";
import { readCockpitSettings } from "../state/cockpit-settings";
import type { CockpitInboxItem } from "../state/workspace-cockpit-projection";
import {
  cockpitInbox,
  cockpitObservedRecords,
  installCockpitPageTestHooks,
  renderCockpit,
  watchSession,
} from "./ChatCockpitPage.test-utils";

vi.mock("../api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof ApiClient>()),
  findWorkspaceSessionIssueAddress: vi.fn(),
  getAutomationEnrollment: vi.fn(),
  postAutomationEnrollmentTerminate: vi.fn(),
  takeoverWorkspaceSession: vi.fn(),
}));

vi.mock("../state/bulk-confirm-store", () => ({
  useBulkConfirmStore: Object.assign(
    (selector: (state: { runs: readonly unknown[] }) => unknown) =>
      selector({ runs: [] }),
    { getState: () => ({ start: vi.fn() }) },
  ),
}));

vi.mock("../hooks/useWorkspaceWs", async (importOriginal) => ({
  ...(await importOriginal<typeof WorkspaceWsModule>()),
  useWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
}));

vi.mock("../api/workspace-content", () => ({
  fetchWorkspaceArtifactVersion: vi.fn(),
  fetchWorkspaceEventOutput: vi.fn(),
  fetchWorkspaceNodeDetail: vi.fn(),
  fetchWorkspacePrompt: vi.fn(),
}));

vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => (
    <div data-testid="monaco-viewer">{value}</div>
  ),
}));

vi.mock("../components/cockpit/CockpitShell", () => ({
  useCockpitCodingAttemptForSession: () => () => null,
  useCockpitShellInbox: () => (cockpitInbox.length > 0 ? cockpitInbox : []),
  useCockpitInboxPulse: () => false,
  useCockpitSessionWatch: () => watchSession,
  useCockpitSettings: () => readCockpitSettings(),
  useCockpitObservedRecords: () => cockpitObservedRecords,
  useCockpitSettingsSlotRef: () => () => undefined,
  useCockpitAttentionTargetOpener: () => () => undefined,
  useCockpitInboxFocusOpener: () => null,
}));

function inboxItem(overrides: Partial<CockpitInboxItem> & { id: string }): CockpitInboxItem {
  return {
    kind: "gate",
    severity: 3,
    title: "需要人工处理的条目",
    summary: "来自根壳观察",
    triage: false,
    source: "gate",
    createdAt: null,
    gate: null,
    inlineError: null,
    choice: null,
    ...overrides,
  };
}

describe("ChatCockpitPage 待处理徽标计数（F-A 第 3 条）", () => {
  installCockpitPageTestHooks();

  it("info 类条目不计入徽标：1 条真待办 + 2 条进度信息 → 徽标 1", () => {
    cockpitInbox.push(
      inboxItem({ id: "other:gate", kind: "gate" }),
      inboxItem({ id: "lease_wait:auto", kind: "info", source: "c1_waiting" }),
      inboxItem({ id: "generation_history:readonly", kind: "info", source: "gate" }),
    );

    renderCockpit();

    expect(screen.getByTestId("cockpit-inbox-drawer-count")).toHaveTextContent("1");
    // 抽屉里 info 仍然可见（只是不计数）：信息不减，分组不同。
    expect(screen.getByTestId("cockpit-inbox-drawer-trigger")).toHaveTextContent("待处理");
  });

  it("只有 info 类条目时徽标不出现（无待办形态）", () => {
    cockpitInbox.push(
      inboxItem({ id: "lease_wait:auto", kind: "info", source: "c1_waiting" }),
    );

    renderCockpit();

    expect(screen.queryByTestId("cockpit-inbox-drawer-count")).toBeNull();
  });
});
