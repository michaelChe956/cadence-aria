import { useEffect, type ReactNode } from "react";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
import { getProviderOptions } from "../../state/provider-options";

export function ProviderAvailabilityGuard({ children }: { children: ReactNode }) {
  const loadStatus = useProviderAvailabilityStore((state) => state.loadStatus);
  const snapshot = useProviderAvailabilityStore((state) => state.snapshot);
  const realWorkflowBlocked = useProviderAvailabilityStore(
    (state) => state.realWorkflowBlocked,
  );
  const stateStatus = useProviderAvailabilityStore((state) => state.stateStatus);
  const stateError = useProviderAvailabilityStore((state) => state.stateError);
  const recheckStatus = useProviderAvailabilityStore((state) => state.recheckStatus);
  const error = useProviderAvailabilityStore((state) => state.error);
  const load = useProviderAvailabilityStore((state) => state.load);
  const recheck = useProviderAvailabilityStore((state) => state.recheck);

  useEffect(() => {
    const state = useProviderAvailabilityStore.getState();
    if (state.loadStatus === "idle" && !state.snapshot) {
      void load();
    }
  }, [load]);

  if (!snapshot && (loadStatus === "idle" || loadStatus === "loading")) {
    return (
      <main
        role="status"
        className="fixed inset-0 grid min-h-screen place-items-center bg-[var(--aria-bg)] p-6 text-[var(--aria-ink)]"
      >
        <div className="text-center">
          <h1 className="text-lg font-semibold">正在检测 Claude Code 与 Codex</h1>
          <p className="mt-2 text-sm text-[var(--aria-ink-muted)]">
            正在确认可用的真实 Provider，请稍候。
          </p>
        </div>
      </main>
    );
  }

  if (snapshot && !realWorkflowBlocked) {
    return (
      <>
        {children}
        {error ? (
          <p
            role="alert"
            className="fixed bottom-4 right-4 z-[100] max-w-md rounded-md border border-[var(--aria-danger)] bg-[var(--aria-panel)] p-3 text-sm text-[var(--aria-danger)] shadow-lg"
          >
            Provider 重新检测失败：{error}
          </p>
        ) : null}
      </>
    );
  }

  const initialLoadFailed = !snapshot && loadStatus === "error";
  const realProviders = snapshot
    ? getProviderOptions(snapshot).filter((option) => option.real)
    : [];
  const rechecking = recheckStatus === "rechecking";

  return (
    <main className="fixed inset-0 z-[100] min-h-screen overflow-y-auto bg-[var(--aria-bg)] p-4 text-[var(--aria-ink)] sm:p-8">
      <section
        role="dialog"
        aria-modal="true"
        aria-labelledby="provider-availability-title"
        aria-describedby="provider-availability-description"
        className="mx-auto my-auto w-full max-w-3xl rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-5 shadow-2xl sm:p-8"
      >
        <header className="border-b border-[var(--aria-line)] pb-5">
          <p className="text-xs font-semibold uppercase tracking-wide text-[var(--aria-danger)]">
            {initialLoadFailed ? "Provider 状态不可读" : "Provider 检测未通过"}
          </p>
          <h1 id="provider-availability-title" className="mt-2 text-2xl font-semibold">
            {initialLoadFailed ? "Provider 状态读取失败" : "需要安装或修复 Provider"}
          </h1>
          <p
            id="provider-availability-description"
            className="mt-2 text-sm leading-6 text-[var(--aria-ink-muted)]"
          >
            {initialLoadFailed
              ? "无法确认 Claude Code 与 Codex 的当前状态。为避免在未知状态下启动真实工作流，应用将保持关闭。"
              : "Claude Code 与 Codex 当前均无法支持真实工作流。完成以下修复后重新检测，应用才会开放。"}
          </p>
        </header>

        {snapshot ? (
          <>
            <div className="mt-5 grid gap-4 sm:grid-cols-2">
              {realProviders.map((provider) => {
                const health = snapshot.providers.find(
                  (entry) => entry.provider === provider.value,
                );
                return (
                  <article
                    key={provider.value}
                    className="rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-4"
                  >
                    <h2 className="font-semibold">{provider.label}</h2>
                    <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">
                      版本：{health?.version ?? "未检测到版本"}
                    </p>
                    <p className="mt-2 text-sm text-[var(--aria-danger)]">
                      {provider.reason ?? "Provider 当前不可用"}
                    </p>
                    {provider.installHint ? (
                      <p className="mt-3 whitespace-pre-wrap break-words text-sm leading-6 text-[var(--aria-ink-muted)]">
                        {provider.installHint}
                      </p>
                    ) : null}
                  </article>
                );
              })}
            </div>

            {stateStatus === "degraded" && stateError ? (
              <p
                role="alert"
                className="mt-5 rounded-md border border-[var(--aria-danger)] bg-[var(--aria-panel-muted)] p-3 text-sm text-[var(--aria-danger)]"
              >
                状态存储降级：{stateError}
              </p>
            ) : null}

            <p className="mt-5 font-mono text-xs text-[var(--aria-ink-muted)]">
              generation {snapshot.generation} · checked_at {snapshot.checked_at}
            </p>
          </>
        ) : null}

        {error ? (
          <p
            role="alert"
            className="mt-5 rounded-md border border-[var(--aria-danger)] bg-[var(--aria-panel-muted)] p-3 text-sm text-[var(--aria-danger)]"
          >
            {initialLoadFailed ? "状态读取失败" : "重新检测失败"}：{error}
          </p>
        ) : null}

        <div className="mt-6 flex justify-end">
          <button
            type="button"
            autoFocus
            disabled={rechecking}
            onClick={() => void recheck()}
            className="inline-flex min-h-10 cursor-pointer items-center justify-center rounded-md bg-[var(--aria-primary)] px-4 text-sm font-semibold text-white transition-colors hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {rechecking ? "正在重新检测" : "重新检测"}
          </button>
        </div>
      </section>
    </main>
  );
}
