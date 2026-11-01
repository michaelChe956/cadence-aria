import { useEffect, useRef } from "react";
import type { WsProviderConfig } from "../api/types";
import { readWorkspaceProviderDefaults } from "../state/workspace-provider-defaults";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import type { WorkspaceWsApi } from "./useWorkspaceWs";

export interface ProviderDefaultsApplicationOptions {
  ws: Pick<WorkspaceWsApi, "selectProvider">;
  sessionId: string;
  connected: boolean;
  providerEditable: boolean;
  providers: WsProviderConfig | null;
  isTakeover: boolean;
}

/**
 * 用户默认 provider 的共享应用机制（REQ-PPS-02）：Cockpit 与 legacy workspace 页同源
 * ——原先只存在于 ChatCockpitPage 的内联补发块迁移至此，legacy 页首次获得该能力。
 *
 * 幂等且克制：每个 session 只在差异存在时补发一次 provider_select；已锁定/非
 * PrepareContext（providerEditable 为 false）、未连接、观察态（takeover）会话都不补发，
 * 不覆盖正在运行的会话；reviewer 仅在用户默认启用交叉审核时补发。
 */
export function useProviderDefaultsApplication({
  ws,
  sessionId,
  connected,
  providerEditable,
  providers,
  isTakeover,
}: ProviderDefaultsApplicationOptions): void {
  const appliedSessionRef = useRef<string | null>(null);
  const storeSessionId = useWorkspaceStore((state) => state.sessionId);

  useEffect(() => {
    if (
      isTakeover ||
      storeSessionId !== sessionId ||
      !providerEditable ||
      !connected ||
      appliedSessionRef.current === sessionId ||
      !providers
    ) {
      return;
    }

    const defaults = readWorkspaceProviderDefaults();
    if (!defaults) {
      return;
    }

    appliedSessionRef.current = sessionId;
    if (providers.author !== defaults.author) {
      ws.selectProvider("author", defaults.author);
    }
    if (defaults.reviewerEnabled && providers.reviewer !== defaults.reviewer) {
      ws.selectProvider("reviewer", defaults.reviewer);
    }
    if (useWorkspaceStore.getState().reviewerEnabled !== defaults.reviewerEnabled) {
      useWorkspaceStore.setState({ reviewerEnabled: defaults.reviewerEnabled });
    }
  }, [
    connected,
    isTakeover,
    providerEditable,
    providers,
    sessionId,
    storeSessionId,
    ws,
  ]);
}
