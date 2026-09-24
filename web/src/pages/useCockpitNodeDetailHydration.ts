import { useEffect, useRef } from "react";
import { fetchWorkspaceNodeDetail } from "../api/workspace-content";
import { loadAcknowledgedAbortedNodes } from "../components/workspace/DisconnectBanner";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import type { TimelineNode } from "../state/workspace-ws-store-types";

/**
 * F-47 REQ-NDR-02：detail 水合集合 = 呈现节点里的「非运行中」节点。
 *
 * 判定用排除法（非 active、非 paused）而非枚举 completed：failed/skipped 以及
 * spec 点名的 aborted/interrupted（前端状态 union 尚未枚举）这类终态节点此前
 * 全被排除在水合之外，即便 durable 里已有 usage，其 token 行也永不显示
 * （诊断 §2.2 B3）。仍维持按需语义：只水合调用方传入的呈现节点，不做全量预取；
 * paused 未终态（durable detail 尚未产生）同样排除在集合外。
 */
export function detailHydrationNodeIds(timelineNodes: TimelineNode[]): string[] {
  return timelineNodes
    .filter((node) => node.status !== "active" && node.status !== "paused")
    .map((node) => node.node_id);
}

/**
 * cockpit 节点 detail 水合 effect 族（自 ChatCockpitPage 机械拆出，纯移动零行为
 * 变化）：v40 复验 #3 水合去重 ref + 会话切换清空 + 终态/active 节点 detail
 * 拉取 + 断连中止节点确认态恢复，effect 体与依赖数组原样迁移。
 */
export function useCockpitNodeDetailHydration({
  sessionId,
  timelineNodes,
  activeNodeId,
}: {
  sessionId: string;
  timelineNodes: TimelineNode[];
  activeNodeId: string | null;
  // v40 复验 #3 后续（刷新水合）：review verdict 只随节点 detail 携带（
  // /timeline-node-details/{id}），live 时 WS 事件入 store；刷新/重开后 cockpit
  // 页此前无 detail 水合（仅 Legacy 页有同款 effect），review_verdict 条目无法
  // 重建——主区/收件箱「采纳 Review 意见」与审核结论卡在刷新后整体消失。对齐
  // Legacy 纪律：拉取全部已完成节点 detail（气泡 usage 行同源受益）。
}): void {
  // v40 复验 #3：节点 detail 水合去重（见下方水合 effect）。
  const hydratedNodeIdsRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    hydratedNodeIdsRef.current.clear();
  }, [sessionId]);
  useEffect(() => {
    const nodeIds = Array.from(
      new Set(
        [activeNodeId, ...detailHydrationNodeIds(timelineNodes)].filter(
          (nodeId): nodeId is string => typeof nodeId === "string" && nodeId.length > 0,
        ),
      ),
    );
    for (const nodeId of nodeIds) {
      if (hydratedNodeIdsRef.current.has(nodeId)) {
        continue;
      }
      hydratedNodeIdsRef.current.add(nodeId);
      Promise.resolve(fetchWorkspaceNodeDetail(sessionId, nodeId))
        .then((detail) => {
          if (!detail) {
            hydratedNodeIdsRef.current.delete(nodeId);
            return;
          }
          const current = useWorkspaceStore.getState();
          if (current.sessionId !== sessionId) {
            return;
          }
          current.setNodeDetail(detail);
        })
        .catch(() => {
          hydratedNodeIdsRef.current.delete(nodeId);
        });
    }
  }, [sessionId, activeNodeId, timelineNodes]);
  useEffect(() => {
    const acknowledgedNodes = loadAcknowledgedAbortedNodes();
    if (acknowledgedNodes.length > 0) {
      useWorkspaceStore
        .getState()
        .setAcknowledgedAbortedNodes(acknowledgedNodes);
    }
  }, []);
}
