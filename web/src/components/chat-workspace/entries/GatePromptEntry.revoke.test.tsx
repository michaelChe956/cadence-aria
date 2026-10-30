// REQ-WSC-13/14（k3 P0-2C）门卡「撤销排除」前端面：
// ① 已排除行「撤销排除」→ 门面 revokeFindingExclusions（REST，会话级 durable
//    事实）；② 成功后徽章消失、「排除此项」回来、门卡「已排除 N 项」计数同步、
//    即时反馈可见；③ 失败不冒充成功——徽章保留，后端 code/message 如实上屏；
// ④ 在途禁用（「撤销中…」）防重复出站；⑤ 刷新后经空 key 只读引导恢复 durable
//    已排除态，仍可撤销；⑥ 多条已排除时「全部撤销」一次清空本卡。
// 独立文件：GatePromptEntry.test.tsx 已 1151+ 行，行数门禁（1200）余量不足。
import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import type { RevokeFindingExclusionsOutcome } from "../../../state/workspace-finding-exclusions";
import { installWorkspaceStoreTestHooks } from "../../../state/workspace-ws-store.test-utils";
import { GatePromptEntry } from "./GatePromptEntry";

const KEY_A = "aaaaaaaaaaaaaaaa";
const KEY_B = "bbbbbbbbbbbbbbbb";

function gateEntry(metadata: Record<string, unknown> = {}): ChatEntry {
  return {
    id: "gate:entry",
    type: "gate_prompt",
    role: "system",
    content: "等待人工确认",
    timestamp: "2026-09-17T00:00:00Z",
    metadata: {
      action_facade: "typed",
      command_id: "cmd_1",
      action_block_reason: null,
      ...metadata,
    },
  };
}

function findingsEntry(): ChatEntry {
  return gateEntry({
    findings: [
      { severity: "must_fix", message: "问题 A", findingKey: KEY_A },
      { severity: "must_fix", message: "问题 B", findingKey: KEY_B },
    ],
  });
}

function okOutcome(remaining: string[]): RevokeFindingExclusionsOutcome {
  return {
    ok: true,
    result: {
      command_id: "cmd-revoke",
      state: "accepted",
      remaining_finding_keys: remaining,
    },
  };
}

function failedOutcome(): RevokeFindingExclusionsOutcome {
  return {
    ok: false,
    code: "human_action_rejected",
    message: "disposition ledger unavailable",
  };
}

type RevokeSender = (findingKeys: string[]) => Promise<RevokeFindingExclusionsOutcome>;

function actions(revoke?: RevokeSender): CockpitActionFacade {
  return {
    confirm: vi.fn(() => true),
    confirmReview: vi.fn(() => true),
    feedback: vi.fn(() => true),
    terminate: vi.fn(() => true),
    advance: vi.fn(() => true),
    adoptReview: vi.fn(() => true),
    confirmBatch: vi.fn(async () => undefined),
    recoverCompile: vi.fn(async () => undefined),
    recoverCandidate: vi.fn(),
    retryInitialization: vi.fn(async () => undefined),
    confirmTakeover: vi.fn(async () => undefined),
    rebind: vi.fn(),
    sendBootstrapAction: vi.fn(),
    ...(revoke ? { revokeFindingExclusions: vi.fn(revoke) } : {}),
  };
}

/** 空引导（刷新读 durable）按需返回清单；显式撤销按剩余清单回。 */
function sender(script: {
  bootstrapRemaining?: string[];
  onRevoke?: (
    keys: string[],
  ) => RevokeFindingExclusionsOutcome | Promise<RevokeFindingExclusionsOutcome>;
}): { send: RevokeSender; calls: string[][] } {
  const calls: string[][] = [];
  const send: RevokeSender = async (keys) => {
    calls.push(keys);
    if (keys.length === 0) {
      return okOutcome(script.bootstrapRemaining ?? []);
    }
    return script.onRevoke?.(keys) ?? okOutcome([]);
  };
  return { send, calls };
}

async function excludeFirstFinding(user: UserEvent) {
  await user.click(screen.getAllByTestId("finding-exclude-button")[0]);
}

async function excludeBothFindings(user: UserEvent) {
  await user.click(screen.getAllByTestId("finding-exclude-button")[0]);
  await user.click(screen.getAllByTestId("finding-exclude-button")[0]);
}

describe("REQ-WSC-13/14 gate card finding exclusion revocation", () => {
  installWorkspaceStoreTestHooks();

  it("revokes one exclusion: badge disappears, exclude button returns, count syncs, notice shows", async () => {
    const user = userEvent.setup();
    const { send, calls } = sender({
      onRevoke: (keys) => okOutcome(keys.length === 1 ? [KEY_B] : []),
    });
    render(<GatePromptEntry entry={findingsEntry()} actions={actions(send)} />);

    await excludeBothFindings(user);
    expect(screen.getByTestId("gate-finding-excluded-note").textContent).toContain(
      "已排除 2 项",
    );

    await user.click(screen.getAllByTestId("finding-revoke-exclusion-button")[0]);

    expect(calls.at(-1)).toEqual([KEY_A]);
    // 行级：徽章消失、「排除此项」回来（第一条行 = index 0）。
    await waitFor(() => {
      expect(screen.getAllByTestId("finding-exclude-button")).toHaveLength(1);
    });
    expect(screen.getAllByTestId("finding-excluded-badge")).toHaveLength(1);
    // 门卡计数同步减一 + 即时反馈。
    expect(screen.getByTestId("gate-finding-excluded-note").textContent).toContain(
      "已排除 1 项",
    );
    expect(screen.getByTestId("gate-finding-revoke-notice").textContent).toContain(
      "已撤销排除，这条下一轮会重新进入评审",
    );
    expect(screen.queryByTestId("gate-finding-revoke-failed")).toBeNull();
  });

  it("keeps the exclusion and surfaces the backend reason when the revoke fails", async () => {
    const user = userEvent.setup();
    const { send } = sender({ onRevoke: () => failedOutcome() });
    render(<GatePromptEntry entry={findingsEntry()} actions={actions(send)} />);

    await excludeFirstFinding(user);
    await user.click(screen.getAllByTestId("finding-revoke-exclusion-button")[0]);

    // 失败不得冒充成功：徽章保留、计数不减、原因如实上屏。
    expect(screen.getAllByTestId("finding-excluded-badge")).toHaveLength(1);
    expect(screen.getByTestId("gate-finding-excluded-note").textContent).toContain(
      "已排除 1 项",
    );
    expect(screen.queryByTestId("gate-finding-revoke-notice")).toBeNull();
    const failure = screen.getByTestId("gate-finding-revoke-failed");
    expect(failure.textContent).toContain("human_action_rejected");
    expect(failure.textContent).toContain("disposition ledger unavailable");
  });

  it("disables the row button with「撤销中…」while the revoke is in flight", async () => {
    const user = userEvent.setup();
    let resolveRevoke: ((outcome: RevokeFindingExclusionsOutcome) => void) | null = null;
    const { send } = sender({
      onRevoke: () =>
        new Promise<RevokeFindingExclusionsOutcome>((resolve) => {
          resolveRevoke = resolve;
        }),
    });
    render(<GatePromptEntry entry={findingsEntry()} actions={actions(send)} />);

    await excludeFirstFinding(user);
    await user.click(screen.getAllByTestId("finding-revoke-exclusion-button")[0]);

    const inFlightButton = screen.getAllByTestId("finding-revoke-exclusion-button")[0];
    expect(inFlightButton).toBeDisabled();
    expect(inFlightButton.textContent).toContain("撤销中…");

    resolveRevoke!(okOutcome([]));
    await waitFor(() => {
      expect(screen.queryByTestId("finding-revoke-exclusion-button")).toBeNull();
    });
    expect(screen.getAllByTestId("finding-exclude-button")).toHaveLength(2);
  });

  it("restores durable exclusions after refresh via the empty-key read and can revoke them", async () => {
    const user = userEvent.setup();
    const { send, calls } = sender({
      bootstrapRemaining: [KEY_A],
      onRevoke: () => okOutcome([]),
    });
    render(<GatePromptEntry entry={findingsEntry()} actions={actions(send)} />);

    // 刷新后：未点过任何按钮，durable 引导（空 key 只读）即恢复「已排除」。
    expect(await screen.findAllByTestId("finding-excluded-badge")).toHaveLength(1);
    expect(screen.getByTestId("gate-finding-excluded-note").textContent).toContain(
      "已排除 1 项",
    );
    expect(calls[0]).toEqual([]);

    await user.click(screen.getAllByTestId("finding-revoke-exclusion-button")[0]);

    expect(calls.at(-1)).toEqual([KEY_A]);
    await waitFor(() => {
      expect(screen.queryByTestId("finding-excluded-badge")).toBeNull();
    });
    expect(screen.getAllByTestId("finding-exclude-button")).toHaveLength(2);
  });

  it("revokes all exclusions on this card in one call and clears the note", async () => {
    const user = userEvent.setup();
    const { send, calls } = sender({ onRevoke: () => okOutcome([]) });
    render(<GatePromptEntry entry={findingsEntry()} actions={actions(send)} />);

    await excludeBothFindings(user);
    const revokeAll = screen.getByTestId("gate-revoke-all-exclusions");
    expect(revokeAll.textContent).toContain("全部撤销");

    await user.click(revokeAll);

    expect(calls.at(-1)).toEqual([KEY_A, KEY_B]);
    await waitFor(() => {
      expect(screen.queryByTestId("gate-finding-excluded-note")).toBeNull();
    });
    expect(screen.getAllByTestId("finding-exclude-button")).toHaveLength(2);
    expect(screen.queryByTestId("finding-revoke-exclusion-button")).toBeNull();
    expect(screen.getByTestId("gate-finding-revoke-notice").textContent).toContain(
      "已撤销全部排除（2 项）",
    );
  });

  it("does not render revoke entries when the facade does not provide the optional face (fail-closed)", async () => {
    const user = userEvent.setup();
    render(<GatePromptEntry entry={findingsEntry()} actions={actions()} />);

    await excludeFirstFinding(user);

    expect(screen.getAllByTestId("finding-excluded-badge")).toHaveLength(1);
    expect(screen.queryByTestId("finding-revoke-exclusion-button")).toBeNull();
    expect(screen.queryByTestId("gate-revoke-all-exclusions")).toBeNull();
  });
});
