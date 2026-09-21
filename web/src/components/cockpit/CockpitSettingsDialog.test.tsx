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

  it("lays each reminder-layer checkbox left of its label inside a full-row hit area", async () => {
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

    const checkbox = screen.getByLabelText("声音提醒");
    const labelText = screen.getByText("声音提醒");
    expect(
      checkbox.compareDocumentPosition(labelText) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();

    await user.click(screen.getByText("标题 emoji"));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ titleEmojiEnabled: false }),
    );
  });

  it("toggles an autopilot stop point by clicking its row label", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <CockpitSettingsDialog
        open
        onClose={vi.fn()}
        settings={settings({ stopPoints: ["human_gate"] })}
        onChange={onChange}
      />,
    );

    await user.click(screen.getByText("硬错误"));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ stopPoints: ["human_gate", "hard_error"] }),
    );
  });

  it("renders one decorative chevron per select, hidden from assistive tech", () => {
    render(
      <CockpitSettingsDialog
        open
        onClose={vi.fn()}
        settings={settings()}
        onChange={vi.fn()}
      />,
    );

    const selects = screen.getAllByRole("combobox");
    expect(selects).toHaveLength(6);
    for (const select of selects) {
      const field = select.closest("label");
      expect(field).not.toBeNull();
      expect(field?.querySelectorAll("svg[aria-hidden='true']")).toHaveLength(1);
    }
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
