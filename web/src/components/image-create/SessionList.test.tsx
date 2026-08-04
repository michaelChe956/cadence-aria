import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionSummary } from "../../api/types/image-create";
import { useImageCreateStore } from "../../state/image-create-store";
import { SessionList } from "./SessionList";

const originalState = useImageCreateStore.getState();
const sessions: SessionSummary[] = [
  {
    id: "session-1",
    provider_name: "claude_code",
    template: { preset: "ppt_business_illustration" },
    status: "active",
    created_at: "2026-08-03T09:00:00Z",
    updated_at: "2026-08-03T09:00:00Z",
  },
  {
    id: "session-2",
    provider_name: "codex",
    template: { custom: "极简线稿" },
    status: "deleting",
    created_at: "2026-08-03T08:00:00Z",
    updated_at: "2026-08-03T08:00:00Z",
  },
];

beforeEach(() => {
  useImageCreateStore.setState({
    ...originalState,
    sessions,
    currentSession: null,
  });
});

async function selectCustomOption(
  user: ReturnType<typeof userEvent.setup>,
  label: string,
  option: string,
) {
  await user.click(screen.getByRole("combobox", { name: label }));
  await user.click(screen.getByRole("option", { name: option }));
}

describe("SessionList", () => {
  it("creates preset and custom sessions with the selected provider", async () => {
    const user = userEvent.setup();
    const createSession = vi.fn().mockResolvedValue({});
    useImageCreateStore.setState({ createSession });
    render(<SessionList />);

    await user.click(screen.getByRole("button", { name: "新建会话" }));
    await selectCustomOption(user, "模板", "业务流程图");
    await selectCustomOption(user, "Provider", "codex");
    await user.click(screen.getByRole("button", { name: "创建" }));
    expect(createSession).toHaveBeenCalledWith(
      { preset: "business_flow_diagram" },
      "codex",
    );

    await user.click(screen.getByRole("button", { name: "新建会话" }));
    await selectCustomOption(user, "模板", "自定义引导词");
    await user.type(screen.getByLabelText("自定义引导词"), "科技感海报");
    await user.click(screen.getByRole("button", { name: "创建" }));
    expect(createSession).toHaveBeenCalledWith({ custom: "科技感海报" }, "claude_code");
  });

  it("handles a rejected open-session promise at the click boundary", async () => {
    const rejected = Promise.reject(new Error("打开失败"));
    void rejected.catch(() => {});
    const catchSpy = vi.spyOn(rejected, "catch");
    useImageCreateStore.setState({ openSession: vi.fn(() => rejected) });

    render(<SessionList />);
    fireEvent.click(screen.getByRole("button", { name: /PPT 商务配图/ }));

    await waitFor(() => expect(catchSpy).toHaveBeenCalledTimes(1));
  });

  it("switches and deletes sessions", async () => {
    const openSession = vi.fn().mockResolvedValue(undefined);
    const deleteSession = vi.fn().mockResolvedValue(undefined);
    useImageCreateStore.setState({ openSession, deleteSession });
    render(<SessionList />);

    fireEvent.click(screen.getByRole("button", { name: /PPT 商务配图/ }));
    expect(openSession).toHaveBeenCalledWith("session-1");

    fireEvent.click(screen.getByRole("button", { name: "删除会话 session-1" }));
    await waitFor(() => expect(deleteSession).toHaveBeenCalledWith("session-1"));
  });
});
