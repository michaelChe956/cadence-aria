import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type * as WorkspaceWsModule from "../hooks/useWorkspaceWs";
import type * as ApiClient from "../api/client";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { selectCockpitInbox } from "../state/workspace-cockpit-projection";
import { readCockpitSettings } from "../state/cockpit-settings";
import type { ChatEntry } from "../state/chat-entries";
import { ChatCockpitPage } from "./ChatCockpitPage";
import {
  cockpitInbox,
  cockpitObservedRecords,
  installCockpitPageTestHooks,
  renderCockpit,
  watchSession,
} from "./ChatCockpitPage.test-utils";

vi.mock("../hooks/useWorkspaceWs", async (importOriginal) => ({
  ...(await importOriginal<typeof WorkspaceWsModule>()),
  useWorkspaceWs: vi.fn(),
}));
vi.mock("../hooks/useUnloadGuard", () => ({ useUnloadGuard: vi.fn() }));
vi.mock("../api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof ApiClient>()),
  takeoverWorkspaceSession: vi.fn(),
}));
vi.mock("../api/workspace-content", () => ({
  fetchWorkspaceArtifactVersion: vi.fn(),
  fetchWorkspaceEventOutput: vi.fn(),
  fetchWorkspaceNodeDetail: vi.fn(),
  fetchWorkspacePrompt: vi.fn(),
}));
vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => <div data-testid="monaco-viewer">{value}</div>,
}));
vi.mock("../components/cockpit/CockpitShell", () => ({
  useCockpitCodingAttemptForSession: () => () => null,
  useCockpitShellInbox: () =>
    cockpitInbox.length > 0
      ? cockpitInbox
      : selectCockpitInbox(useWorkspaceStore.getState()).map((item) => ({
          ...item,
          id: `${useWorkspaceStore.getState().sessionId}:${item.id}`,
        })),
  useCockpitInboxPulse: () => false,
  useCockpitSessionWatch: () => watchSession,
  useCockpitSettings: () => readCockpitSettings(),
  useCockpitObservedRecords: () => cockpitObservedRecords,
  useCockpitSettingsSlotRef: () => () => undefined,
}));

const DANGEROUS_PROMPT = '⚠️ Dangerous command:\n\n  rm -rf "$BASE"\n\nAllow?';

function cockpitChoiceEntry(overrides: Partial<ChatEntry> = {}): ChatEntry {
  return {
    id: "choice_request:choice-1",
    type: "choice_request",
    role: "author",
    content: DANGEROUS_PROMPT,
    timestamp: "2026-09-23T04:00:45Z",
    metadata: {
      request_id: "choice-1",
      prompt: DANGEROUS_PROMPT,
      source: "provider_choice",
      options: [
        { id: "Yes", label: "Yes" },
        { id: "No", label: "No" },
      ],
      allow_multiple: false,
      allow_free_text: true,
    },
    ...overrides,
  } as ChatEntry;
}

describe("ChatCockpitPage choice arrival notice", () => {
  installCockpitPageTestHooks();

  // F-43 ②：cockpit 页的待处理提示此前只挂在 observer/收件箱（选择帧不进
  // observer），对话流里新到的未处理 choice 没有任何提示。契约：对话流区内
  // 常驻横幅 + 定位到卡，应答后消失。
  it("announces a pending choice inside the conversation flow and jumps to the card", async () => {
    useWorkspaceStore.setState({ chatEntries: [cockpitChoiceEntry()] });

    renderCockpit();

    const conversation = screen.getByTestId("cockpit-conversation-flow");
    const notice = screen.getByTestId("pending-choice-notice");
    expect(conversation).toContainElement(notice);
    expect(notice).toHaveTextContent("有 1 个选择请求待处理");
    expect(notice).toHaveTextContent("⚠️ Dangerous command:");

    await userEvent.click(screen.getByRole("button", { name: "定位选择卡" }));

    expect(screen.getByTestId("choice-request-entry")).toBeInTheDocument();
  });

  it("drops the announcement once the pending choice is answered", () => {
    useWorkspaceStore.setState({ chatEntries: [cockpitChoiceEntry()] });

    renderCockpit();
    expect(screen.getByTestId("pending-choice-notice")).toBeInTheDocument();

    act(() => {
      useWorkspaceStore.setState({
        chatEntries: [
          cockpitChoiceEntry({
            resolved: true,
            metadata: {
              request_id: "choice-1",
              prompt: DANGEROUS_PROMPT,
              response: { selected_option_ids: ["Yes"], free_text: null },
            },
          }),
        ],
      });
    });

    expect(screen.queryByTestId("pending-choice-notice")).toBeNull();
  });

  it("stays quiet for resolved choices in the conversation history", () => {
    useWorkspaceStore.setState({
      chatEntries: [
        cockpitChoiceEntry({
          resolved: true,
          metadata: { request_id: "choice-1", prompt: DANGEROUS_PROMPT },
        }),
      ],
    });

    renderCockpit();

    expect(screen.queryByTestId("pending-choice-notice")).toBeNull();
  });
});
