import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MaskedSettings } from "../../api/types/image-create";
import { useImageCreateStore } from "../../state/image-create-store";
import { SettingsDialog } from "./SettingsDialog";

const originalState = useImageCreateStore.getState();
const settings: MaskedSettings = {
  base_url: "https://api.openai.com/v1",
  api_key_masked: "sk-****1234",
  defaults: {
    size: "1024x1024",
    quality: "medium",
    background: "opaque",
    output_format: "png",
  },
};

function renderDialog() {
  const loadSettings = vi.fn(async () => {
    useImageCreateStore.setState({ settings });
  });
  const saveSettings = vi.fn().mockResolvedValue(undefined);
  useImageCreateStore.setState({ loadSettings, saveSettings });

  render(<SettingsDialog onClose={vi.fn()} />);

  return { loadSettings, saveSettings };
}

beforeEach(() => {
  useImageCreateStore.setState({
    ...originalState,
    settings: null,
    error: null,
  });
});

describe("SettingsDialog", () => {
  it("loads settings and displays the masked API key", async () => {
    const { loadSettings } = renderDialog();

    expect(await screen.findByDisplayValue("sk-****1234")).toBeInTheDocument();
    expect(screen.getByLabelText("base_url")).toHaveValue(
      "https://api.openai.com/v1",
    );
    expect(loadSettings).toHaveBeenCalledTimes(1);
  });

  it("retains an unchanged masked key and reloads settings after save", async () => {
    const user = userEvent.setup();
    const { loadSettings, saveSettings } = renderDialog();
    await screen.findByDisplayValue("sk-****1234");

    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith({ api_key_action: "retain" });
      expect(loadSettings).toHaveBeenCalledTimes(2);
    });
    expect(saveSettings.mock.calls[0]?.[0]).not.toHaveProperty("api_key");
    expect(screen.getByText("设置已保存")).toBeInTheDocument();
  });

  it("retains the key when editing restores the masked placeholder", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    const apiKey = await screen.findByLabelText("API Key");

    await user.clear(apiKey);
    await user.type(apiKey, "temporary-key");
    await user.clear(apiKey);
    await user.type(apiKey, settings.api_key_masked);
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith({ api_key_action: "retain" });
    });
    expect(saveSettings.mock.calls[0]?.[0]).not.toHaveProperty("api_key");
  });

  it("sends clear without an API key when the clear button is used", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    await screen.findByDisplayValue("sk-****1234");

    await user.click(screen.getByRole("button", { name: "清除 key" }));
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith({ api_key_action: "clear" });
    });
    expect(saveSettings.mock.calls[0]?.[0]).not.toHaveProperty("api_key");
  });

  it("replaces the API key with a newly entered value", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    const apiKey = await screen.findByLabelText("API Key");

    await user.clear(apiKey);
    await user.type(apiKey, "sk-new-secret");
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith({
        api_key_action: "replace",
        api_key: "sk-new-secret",
      });
    });
  });

  it("submits only changed base_url and defaults options", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    await screen.findByDisplayValue("sk-****1234");

    await user.clear(screen.getByLabelText("base_url"));
    await user.type(screen.getByLabelText("base_url"), "http://127.0.0.1:8080/v1");
    await user.selectOptions(screen.getByLabelText("默认质量"), "high");
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    await waitFor(() => {
      expect(saveSettings).toHaveBeenCalledWith({
        base_url: "http://127.0.0.1:8080/v1",
        api_key_action: "retain",
        defaults: {
          ...settings.defaults,
          quality: "high",
        },
      });
    });
  });

  it("blocks an invalid base_url before calling the store", async () => {
    const user = userEvent.setup();
    const { saveSettings } = renderDialog();
    await screen.findByDisplayValue("sk-****1234");

    await user.clear(screen.getByLabelText("base_url"));
    await user.type(screen.getByLabelText("base_url"), "http://example.com/v1");
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    expect(
      await screen.findByText("base_url 必须使用 HTTPS，或使用 localhost/loopback IP"),
    ).toBeInTheDocument();
    expect(saveSettings).not.toHaveBeenCalled();
  });
});
