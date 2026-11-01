import { useCallback, useEffect, useRef, useState } from "react";
import { getCodingAttemptSnapshot, listVerificationTriageRecords } from "../api/client";
import type {
  CodingAttemptAddress,
  CodingAttemptSnapshotResponse,
} from "../api/types";
import { subscribeToLifecycleInvalidation } from "../state/lifecycle-workbench-store";

/**
 * UI-3（待处理统一抽屉）：attention 目录项的操作详情加载器——
 * 「attention 是目录，snapshot 是操作详情」（设计 §3）：打开/选中时按
 * **完整 Attempt 地址**（project/issue/attempt，取自事项投影，不用
 * 「当前 session 最新活跃 Attempt」捷径）拉 `getCodingAttemptSnapshot`；
 * 同 Attempt 复用一次；分诊卡 `expected_version` 只取
 * `listVerificationTriageRecords` 的服务端 `attempt_version`（拉取失败
 * 为 null，fail-closed 不用 rework_count 冒充）。回执后走既有 lifecycle
 * invalidation 总线：任何失效事件清空缓存，展开中的行按需重拉；拉取失败
 * 保留目录项、状态标 error 由调用方就地报错。
 */
export type InboxAttemptDetail = {
  status: "loading" | "ready" | "error";
  snapshot: CodingAttemptSnapshotResponse | null;
  attemptVersion: number | null;
  error: string | null;
};

export type InboxAttemptDetailsApi = {
  /** 幂等加载：loading/ready 状态直接复用（同 Attempt 只拉一次）；error 可重试。 */
  ensure: (address: CodingAttemptAddress) => void;
  get: (attemptId: string) => InboxAttemptDetail | undefined;
  /** 重试失败的拉取（目录项保留，就地报错后的显式重试）。 */
  retry: (attemptId: string) => void;
};

export function useInboxAttemptDetails(): InboxAttemptDetailsApi {
  const [details, setDetails] = useState<Record<string, InboxAttemptDetail>>({});
  // 同步镜像：幂等判定/重试需要读当前态，避免 effect 竞态重复拉取。
  const detailsRef = useRef<Record<string, InboxAttemptDetail>>({});
  const addressesRef = useRef(new Map<string, CodingAttemptAddress>());
  const loadSeqRef = useRef(new Map<string, number>());

  const applyDetail = useCallback((attemptId: string, next: InboxAttemptDetail) => {
    detailsRef.current = { ...detailsRef.current, [attemptId]: next };
    setDetails(detailsRef.current);
  }, []);

  const load = useCallback(
    (address: CodingAttemptAddress, keepStale: boolean) => {
      const attemptId = address.attemptId;
      addressesRef.current.set(attemptId, address);
      const seq = (loadSeqRef.current.get(attemptId) ?? 0) + 1;
      loadSeqRef.current.set(attemptId, seq);
      const previous = detailsRef.current[attemptId] ?? null;
      applyDetail(attemptId, {
        status: "loading",
        // 重试/失效重拉期间保留旧数据供展示（不闪空）。
        snapshot: keepStale ? previous?.snapshot ?? null : null,
        attemptVersion: keepStale ? previous?.attemptVersion ?? null : null,
        error: null,
      });
      void Promise.all([
        getCodingAttemptSnapshot(address),
        // 版本事实与快照并行拉取；版本端点失败不拖垮 choice 详情（仅分诊卡 fail-closed）。
        listVerificationTriageRecords(address)
          .then((response) => response.attempt_version)
          .catch(() => null),
      ])
        .then(([snapshot, attemptVersion]) => {
          if (loadSeqRef.current.get(attemptId) !== seq) {
            return;
          }
          applyDetail(attemptId, {
            status: "ready",
            snapshot,
            attemptVersion,
            error: null,
          });
        })
        .catch((error: unknown) => {
          if (loadSeqRef.current.get(attemptId) !== seq) {
            return;
          }
          applyDetail(attemptId, {
            status: "error",
            snapshot: null,
            attemptVersion: null,
            error:
              error instanceof Error && error.message !== ""
                ? error.message
                : "操作详情拉取失败",
          });
        });
    },
    [applyDetail],
  );

  const ensure = useCallback(
    (address: CodingAttemptAddress) => {
      const existing = detailsRef.current[address.attemptId];
      if (existing && (existing.status === "loading" || existing.status === "ready")) {
        return;
      }
      load(address, false);
    },
    [load],
  );

  const retry = useCallback(
    (attemptId: string) => {
      const address = addressesRef.current.get(attemptId);
      if (!address) {
        return;
      }
      load(address, false);
    },
    [load],
  );

  // 回执后走既有 lifecycle invalidation：失效事件清空缓存；展开中的行
  // （状态从 ready 变 undefined）经自身 effect 重新 ensure，同 Attempt 仍只
  // 复用一次在途请求。
  useEffect(
    () =>
      subscribeToLifecycleInvalidation(() => {
        if (Object.keys(detailsRef.current).length === 0) {
          return;
        }
        loadSeqRef.current = new Map();
        detailsRef.current = {};
        setDetails({});
      }),
    [],
  );

  const get = useCallback(
    (attemptId: string) => details[attemptId],
    [details],
  );

  return { ensure, get, retry };
}
