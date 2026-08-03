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
