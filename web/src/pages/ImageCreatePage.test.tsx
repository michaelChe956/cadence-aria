import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useImageCreateStore } from "../state/image-create-store";
import { ImageCreatePage } from "./ImageCreatePage";

const originalState = useImageCreateStore.getState();

beforeEach(() => {
  useImageCreateStore.setState({
    ...originalState,
    sessions: [],
    currentSession: null,
    entries: [],
    referenceImage: null,
  });
});

describe("ImageCreatePage", () => {
  it("assembles the image creation workspace and loads its initial data", async () => {
    const loadSessions = vi.fn().mockResolvedValue(undefined);
    const loadSettings = vi.fn().mockResolvedValue(undefined);
    const openSession = vi.fn().mockResolvedValue(undefined);
    const disconnect = vi.fn();
    useImageCreateStore.setState({
      loadSessions,
      loadSettings,
      openSession,
      disconnect,
    });

    const { unmount } = render(<ImageCreatePage sessionId="session-1" />);

    expect(screen.getByRole("main", { name: "图片创作" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "会话" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "创作对话" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "建议提示词" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "生成参数" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "参考图" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "设置" })).toBeInTheDocument();

    await waitFor(() => {
      expect(loadSessions).toHaveBeenCalledTimes(1);
      expect(loadSettings).toHaveBeenCalledTimes(1);
      expect(openSession).toHaveBeenCalledWith("session-1");
    });

    unmount();
    expect(disconnect).toHaveBeenCalledTimes(1);
  });

  it("opens the mobile session drawer and closes it after selecting a session", async () => {
    const user = userEvent.setup();
    const openSession = vi.fn().mockResolvedValue(undefined);
    useImageCreateStore.setState({
      sessions: [
        {
          id: "session-1",
          provider_name: "claude_code",
          template: { preset: "ppt_business_illustration" },
          status: "active",
          created_at: "2026-08-03T09:00:00Z",
          updated_at: "2026-08-03T09:00:00Z",
        },
      ],
      openSession,
    });

    render(<ImageCreatePage />);

    const drawer = screen.getByTestId("image-create-session-drawer");
    expect(drawer).toHaveClass("-translate-x-full", "lg:translate-x-0");

    await user.click(screen.getByRole("button", { name: "打开会话列表" }));
    expect(drawer).toHaveClass("translate-x-0");
    expect(screen.getByRole("button", { name: "关闭会话列表" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /PPT 商务配图/ }));

    expect(openSession).toHaveBeenCalledWith("session-1");
    expect(drawer).toHaveClass("-translate-x-full");
  });

  it("keeps the desktop workspace columns while using a single mobile-first flow", () => {
    render(<ImageCreatePage />);

    expect(screen.getByTestId("image-create-workspace")).toHaveClass(
      "lg:grid",
      "lg:grid-cols-[18rem_minmax(0,1fr)]",
    );
    expect(screen.getByTestId("image-create-main-area")).toHaveClass(
      "xl:grid-cols-[minmax(0,1fr)_22rem]",
    );
    expect(screen.getByRole("button", { name: "打开会话列表" })).toHaveClass(
      "lg:hidden",
      "min-h-11",
    );
  });

  it("opens and closes the settings dialog without saving", async () => {
    const user = userEvent.setup();
    const loadSettings = vi.fn().mockResolvedValue(undefined);
    const saveSettings = vi.fn().mockResolvedValue(undefined);
    useImageCreateStore.setState({ loadSettings, saveSettings });

    render(<ImageCreatePage />);

    await user.click(screen.getByRole("button", { name: "设置" }));
    expect(
      screen.getByRole("dialog", { name: "图片创作设置" }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(
      screen.queryByRole("dialog", { name: "图片创作设置" }),
    ).not.toBeInTheDocument();
    expect(saveSettings).not.toHaveBeenCalled();
  });
});
