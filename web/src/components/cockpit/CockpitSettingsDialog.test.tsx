import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  DEFAULT_COCKPIT_SETTINGS,
  type CockpitSettings,
} from "../../state/cockpit-settings";
import { CockpitSettingsDialog } from "./CockpitSettingsDialog";

class NotificationMock {
  static permission: NotificationPermission = "default";
  static requestPermission = vi.fn<() => Promise<NotificationPermission>>();
}

function settings(overrides: Partial<CockpitSettings> = {}): CockpitSettings {
  return { ...DEFAULT_COCKPIT_SETTINGS, ...overrides };
}

describe("CockpitSettingsDialog", () => {
  beforeEach(() => {
    NotificationMock.permission = "default";
    NotificationMock.requestPermission.mockReset();
    NotificationMock.requestPermission.mockResolvedValue("granted");
    vi.stubGlobal("Notification", NotificationMock);
  });

  it("immediately emits a changed K value", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <CockpitSettingsDialog
        open
        onClose={vi.fn()}
        settings={settings()}
        onChange={onChange}
      />,
    );

    await user.selectOptions(screen.getByLabelText("聚合窗口 K"), "4");

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ watchLimit: 4 }),
    );
  });

  it("requests notification permission when notifications are enabled", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <CockpitSettingsDialog
        open
        onClose={vi.fn()}
        settings={settings({ systemNotificationsEnabled: false })}
        onChange={onChange}
      />,
    );

    await user.click(screen.getByLabelText("系统通知"));

    expect(NotificationMock.requestPermission).toHaveBeenCalledOnce();
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ systemNotificationsEnabled: true }),
    );
  });

  it("shows denied notification recovery guidance while retaining page layers", () => {
    NotificationMock.permission = "denied";
    render(
      <CockpitSettingsDialog
        open
        onClose={vi.fn()}
        settings={settings()}
        onChange={vi.fn()}
      />,
    );

    expect(
      screen.getByText(
        "浏览器已拒绝系统通知；请在站点权限中恢复通知，页面内提醒仍保持开启。",
      ),
    ).toBeVisible();
  });
});
