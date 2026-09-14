import { Settings, X } from "lucide-react";
import { useEffect, useRef, useState, type JSX } from "react";
import type { CockpitSettings, CockpitStopPoint } from "../../state/cockpit-settings";

const STOP_POINT_OPTIONS: ReadonlyArray<{ value: CockpitStopPoint; label: string }> = [
  { value: "human_gate", label: "人工门禁" },
  { value: "stopped", label: "会话停点" },
  { value: "hard_error", label: "硬错误" },
];

const REPEAT_GRADIENT_OPTIONS: ReadonlyArray<{
  value: readonly number[];
  label: string;
}> = [
  { value: [1, 2, 3], label: "第 1、2、3 次" },
  { value: [1, 3, 5], label: "第 1、3、5 次" },
  { value: [1, 2, 4], label: "第 1、2、4 次" },
];

function repeatGradientValue(repeats: readonly number[]): string {
  return repeats.join(",");
}

function notificationPermission(): NotificationPermission | null {
  return typeof Notification === "undefined" ? null : Notification.permission;
}

export function CockpitSettingsDialog({
  open,
  onClose,
  settings,
  onChange,
}: {
  open: boolean;
  onClose(): void;
  settings: CockpitSettings;
  onChange(settings: CockpitSettings): void;
}): JSX.Element | null {
  const [permission, setPermission] = useState(notificationPermission);
  const closeButtonRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    closeButtonRef.current?.focus();
  }, [open]);

  if (!open) {
    return null;
  }

  const update = (patch: Partial<CockpitSettings>) => {
    onChange({ ...settings, ...patch });
  };
  const requestNotificationPermission = () => {
    if (typeof Notification === "undefined" || Notification.permission !== "default") {
      return;
    }
    void Notification.requestPermission().then(setPermission);
  };

  return (
    <div className="fixed inset-0 z-[110] flex items-center justify-center bg-slate-950/40 p-4 backdrop-blur-sm">
      <section
        role="dialog"
        aria-label="驾驶舱设置"
        aria-modal="true"
        className="max-h-[calc(100vh-2rem)] w-full max-w-2xl overflow-y-auto rounded-2xl border border-white/80 bg-[var(--aria-panel)] p-5 shadow-[0_24px_64px_rgba(15,23,42,0.22)]"
      >
        <div className="mb-5 flex items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-[var(--aria-primary-soft)] text-[var(--aria-primary)] shadow-sm">
              <Settings aria-hidden="true" className="h-5 w-5" />
            </span>
            <div>
              <h2 className="text-base font-semibold text-[var(--aria-ink)]">驾驶舱设置</h2>
              <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">
                更改会立即保存并应用到当前驾驶舱。
              </p>
            </div>
          </div>
          <button
            ref={closeButtonRef}
            type="button"
            aria-label="关闭"
            onClick={onClose}
            className="inline-flex h-11 w-11 cursor-pointer items-center justify-center rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] transition-colors duration-200 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
          >
            <X aria-hidden="true" className="h-4 w-4" />
          </button>
        </div>

        <div className="space-y-5 rounded-xl bg-[var(--aria-panel-muted)] p-4">
          <fieldset>
            <legend className="text-sm font-semibold text-[var(--aria-ink)]">提醒层级</legend>
            <div className="mt-2.5 grid gap-3 rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 sm:grid-cols-3">
              <Toggle
                label="声音提醒"
                checked={settings.soundEnabled}
                onChange={(checked) => update({ soundEnabled: checked })}
              />
              <Toggle
                label="系统通知"
                checked={settings.systemNotificationsEnabled}
                onChange={(checked) => {
                  update({ systemNotificationsEnabled: checked });
                  if (checked) {
                    requestNotificationPermission();
                  }
                }}
              />
              <Toggle
                label="标题 emoji"
                checked={settings.titleEmojiEnabled}
                onChange={(checked) => update({ titleEmojiEnabled: checked })}
              />
            </div>
            {permission === "denied" ? (
              <p className="mt-3 rounded-lg border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-3 py-2 text-sm text-[var(--aria-ink)]">
                浏览器已拒绝系统通知；请在站点权限中恢复通知，页面内提醒仍保持开启。
              </p>
            ) : null}
            {permission === "default" ? (
              <div className="mt-3 flex items-center justify-between gap-3 text-xs text-[var(--aria-ink-muted)]">
                <span>浏览器会在您授权后显示系统通知。</span>
                <button
                  type="button"
                  onClick={requestNotificationPermission}
                  className="min-h-11 cursor-pointer rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 font-semibold text-[var(--aria-ink)] transition-colors duration-200 hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
                >
                  请求通知权限
                </button>
              </div>
            ) : null}
          </fieldset>

          <fieldset>
            <legend className="text-sm font-semibold text-[var(--aria-ink)]">聚合与升级</legend>
            <div className="mt-2.5 grid gap-3 rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 sm:grid-cols-2">
              <SettingsSelect
                label="聚合窗口 K"
                value={String(settings.watchLimit)}
                options={Array.from({ length: 32 }, (_, index) => ({
                  value: String(index + 1),
                  label: String(index + 1),
                }))}
                onChange={(value) => update({ watchLimit: Number(value) })}
              />
              <SettingsSelect
                label="观察刷新间隔"
                value={String(settings.observerRefreshIntervalMs)}
                options={[
                  { value: "5000", label: "5 秒" },
                  { value: "15000", label: "15 秒" },
                  { value: "30000", label: "30 秒" },
                  { value: "60000", label: "60 秒" },
                ]}
                onChange={(value) => update({ observerRefreshIntervalMs: Number(value) })}
              />
              <SettingsSelect
                label="L4 门开提醒阈值"
                value={String(settings.gateOpenEscalationMs)}
                options={[
                  { value: "60000", label: "1 分钟" },
                  { value: "300000", label: "5 分钟" },
                  { value: "600000", label: "10 分钟" },
                  { value: "900000", label: "15 分钟" },
                ]}
                onChange={(value) => update({ gateOpenEscalationMs: Number(value) })}
              />
              <SettingsSelect
                label="升级重复间隔"
                value={String(settings.escalationRepeatMs)}
                options={[
                  { value: "60000", label: "1 分钟" },
                  { value: "300000", label: "5 分钟" },
                  { value: "600000", label: "10 分钟" },
                ]}
                onChange={(value) => update({ escalationRepeatMs: Number(value) })}
              />
              <SettingsSelect
                label="预算升级阈值"
                value={String(settings.escalationBudgetThreshold)}
                options={[1, 2, 3].map((value) => ({ value: String(value), label: `剩余 ${value} 轮` }))}
                onChange={(value) => update({ escalationBudgetThreshold: Number(value) })}
              />
              <SettingsSelect
                label="预算重复梯度"
                value={repeatGradientValue(settings.escalationBudgetRepeats)}
                options={REPEAT_GRADIENT_OPTIONS.map((option) => ({
                  value: repeatGradientValue(option.value),
                  label: option.label,
                }))}
                onChange={(value) =>
                  update({ escalationBudgetRepeats: value.split(",").map(Number) })
                }
              />
            </div>
          </fieldset>

          <fieldset>
            <legend className="text-sm font-semibold text-[var(--aria-ink)]">自动推进停点</legend>
            <div className="mt-2.5 grid gap-2 rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 sm:grid-cols-3">
              {STOP_POINT_OPTIONS.map((option) => {
                const checked = settings.stopPoints.includes(option.value);
                return (
                  <label key={option.value} className="flex min-h-11 items-center gap-2 text-sm text-[var(--aria-ink)]">
                    <input
                      type="checkbox"
                      checked={checked}
                      onChange={() =>
                        update({
                          stopPoints: checked
                            ? settings.stopPoints.filter((point) => point !== option.value)
                            : [...settings.stopPoints, option.value],
                        })
                      }
                    />
                    {option.label}
                  </label>
                );
              })}
            </div>
          </fieldset>
        </div>

        <div className="mt-5 flex justify-end">
          <button
            type="button"
            onClick={onClose}
            className="min-h-11 cursor-pointer rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-2.5 text-sm font-semibold text-[var(--aria-ink-muted)] shadow-sm transition-colors duration-200 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
          >
            完成
          </button>
        </div>
      </section>
    </div>
  );
}

function Toggle({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange(checked: boolean): void;
}) {
  return (
    <label className="flex min-h-11 items-center justify-between gap-2 text-sm text-[var(--aria-ink)]">
      {label}
      <input
        type="checkbox"
        aria-label={label}
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
    </label>
  );
}

function SettingsSelect({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: readonly { value: string; label: string }[];
  onChange(value: string): void;
}) {
  return (
    <label className="block text-sm font-semibold text-[var(--aria-ink)]">
      {label}
      <select
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        className="mt-1 block h-11 w-full rounded-lg border border-[var(--aria-line)] bg-white px-3 text-sm font-normal text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </label>
  );
}
