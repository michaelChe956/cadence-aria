import type {
  ProviderConfigSnapshot,
  WorkspaceProviderName,
  WorkItemBatchStatePayload,
  WorkItemDraftCandidatePayload,
  WorkItemPlanArtifactPayload,
  WorkItemPlanCandidateDto,
  WorkItemPlanCompileReportPayload,
  WorkItemPlanContextBlockerPayload,
  WorkItemPlanOutlineCandidatePayload,
} from "../api/types";
import type { ChatEntry, ChatEntryRole } from "../state/chat-entries";
import {
  chatRoleForTimelineNode,
  useWorkspaceStore,
  type ExecutionEvent,
  type ProviderStatus,
  type ReviewVerdict,
  type TimelineNode,
  type TimelineNodeStatus,
} from "../state/workspace-ws-store";
import { workItemPlanArtifactUpdateSummary } from "../state/work-item-plan-artifact-summary";
import { stageChangeContent } from "../state/workspace-stage-labels";

export interface WsServerMessage {
  type: string;
  [key: string]: unknown;
}

export const ACTIVE_PROVIDER_STAGES = new Set(["running", "cross_review", "revision"]);

type WorkspaceWsMessageHandlerOptions = {
  invalidatedPreStageNodeIds: Set<string>;
  scheduleFlush: (nodeId: string) => void;
  streamFlushTimeouts: Record<string, ReturnType<typeof setTimeout>>;
};

export function handleWorkspaceWsMessage(
msg: WsServerMessage,
options: WorkspaceWsMessageHandlerOptions,
) {
const { invalidatedPreStageNodeIds, scheduleFlush, streamFlushTimeouts } = options;
const store = useWorkspaceStore.getState();
  switch (msg.type) {
    case "session_state":
      invalidatedPreStageNodeIds.clear();
      store.setSessionState(msg as never);
      store.rebuildChatEntries();
      break;
    case "stream_chunk":
      {
        const nodeId = msg.node_id as string | null | undefined;
        if (
          typeof nodeId === "string" &&
          invalidatedPreStageNodeIds.has(nodeId)
        ) {
          break;
        }
        const nodeStage = nodeId
          ? store.timelineNodes.find((node) => node.node_id === nodeId)?.stage
          : null;
        const isActiveProviderNode =
          Boolean(store.activeRunId) &&
          typeof nodeId === "string" &&
          nodeId === store.activeNodeId &&
          typeof nodeStage === "string" &&
          ACTIVE_PROVIDER_STAGES.has(nodeStage);
        const hasVisitedProviderStage = store.visitedStages.some((stage) =>
          ACTIVE_PROVIDER_STAGES.has(stage),
        );
        const isPendingInitialProviderNode =
          store.stage === "prepare_context" &&
          !hasVisitedProviderStage &&
          isActiveProviderNode &&
          typeof nodeId === "string" &&
          !invalidatedPreStageNodeIds.has(nodeId);
        const acceptsActiveProviderChunk =
          ACTIVE_PROVIDER_STAGES.has(store.stage) || isPendingInitialProviderNode;
        if (!acceptsActiveProviderChunk) {
          break;
        }
        if (nodeId) {
          const role = entryRoleForNode(
            store,
            nodeId,
            chatEntryRole((msg.role as string | undefined) ?? "author"),
          );
          store.appendStreamChunk(msg.content as string, nodeId);
          store.appendBufferedStreamChunk(msg.content as string, nodeId, role);
          scheduleFlush(nodeId);
        } else {
          store.appendStreamChunk(msg.content as string, nodeId);
          handleStreamChunk(store, msg as never);
        }
      }
      break;
    case "message_complete":
      if (msg.node_id) {
        const nodeId = msg.node_id as string;
        const timeout = streamFlushTimeouts[nodeId];
        if (timeout) {
          clearTimeout(timeout);
          delete streamFlushTimeouts[nodeId];
        }
        store.completeBufferedStream(
          nodeId,
          msg.message_id as string,
          msg.checkpoint_id as string,
        );
        store.finalizeStreamingEntry(resolveStreamEntryId(store, nodeId));
        break;
      }
      store.completeMessage(
        msg.message_id as string,
        msg.checkpoint_id as string,
        msg.node_id as string | null | undefined,
      );
      store.finalizeStreamingEntry(resolveStreamEntryId(store, msg.node_id as string | null | undefined));
      break;
    case "stage_change":
      {
        const nextStage = typeof msg.stage === "string" ? msg.stage : "unknown";
        if (!ACTIVE_PROVIDER_STAGES.has(nextStage) && store.activeNodeId) {
          invalidatedPreStageNodeIds.add(store.activeNodeId);
        }
        store.setStage(nextStage);
        store.appendChatEntry({
          id: chatEntryId("stage_change", `${nextStage}:${store.chatEntries.length}`),
          type: "stage_change",
          role: "system",
          content: stageChangeContent(nextStage),
          timestamp: new Date().toISOString(),
          metadata: { stage: nextStage },
        });
        if (nextStage === "human_confirm") {
          const gatePrompt = gatePromptEntryForState(useWorkspaceStore.getState());
          if (gatePrompt) {
            store.appendChatEntry(gatePrompt);
          }
        }
      }
      break;
    case "artifact_update":
      {
        const version = msg.version as number;
        if (msg.candidate) {
          store.setWorkItemPlanCandidate(msg.candidate as WorkItemPlanCandidateDto);
          store.appendChatEntry({
            id: chatEntryId("artifact_update", `candidate:${String(version)}`),
            type: "artifact_update",
            role: "system",
            content: `Work Item Plan 候选已更新 -> v${version}`,
            timestamp: new Date().toISOString(),
            metadata: {
              version,
              candidate: true,
            },
          });
        } else {
          const workItemPlanArtifact = workItemPlanArtifactFromMessage(msg);
          if (workItemPlanArtifact) {
            store.setWorkItemPlanArtifact(workItemPlanArtifact, version);
            const summary = workItemPlanArtifactUpdateSummary(workItemPlanArtifact, version);
            store.appendChatEntry({
              id: chatEntryId("artifact_update", `${workItemPlanArtifact.type}:${String(version)}`),
              type: "artifact_update",
              role: "system",
              content: summary.content,
              timestamp: new Date().toISOString(),
              metadata: summary.metadata,
            });
          } else {
            const markdown = msg.markdown as string;
            store.setArtifact(markdown, version);
            store.setWorkItemPlanCandidate(null);
            store.setWorkItemPlanArtifact(null);
            store.appendChatEntry({
              id: chatEntryId("artifact_update", String(version)),
              type: "artifact_update",
              role: "system",
              content: `产物已更新 -> v${version}`,
              timestamp: new Date().toISOString(),
              metadata: {
                version,
                diff: (msg as { diff?: string | null }).diff ?? null,
              },
              content_ref:
                version === undefined
                  ? undefined
                  : {
                      kind: "artifact_version",
                      version,
                      sourceNodeId: store.activeNodeId ?? undefined,
                    },
              content_size: markdown.length,
              has_full_content: true,
            });
          }
        }
      }
      break;
    case "permission_request":
      store.addPermissionRequest({
        id: msg.id as string,
        tool_name: msg.tool_name as string,
        description: msg.description as string,
        risk_level: msg.risk_level as "low" | "medium" | "high",
      });
      store.appendChatEntry({
        id: chatEntryId("permission_request", msg.id as string),
        type: "permission_request",
        role: "system",
        content: permissionRequestContent({
          tool_name: msg.tool_name as string,
          description: msg.description as string,
        }),
        timestamp: new Date().toISOString(),
        node_id: store.activeNodeId ?? undefined,
        metadata: {
          request_id: msg.id as string,
          tool_name: msg.tool_name as string,
          description: msg.description as string,
          risk_level: msg.risk_level as "low" | "medium" | "high",
        },
      });
      break;
    case "choice_request":
      store.appendChatEntry({
        id: chatEntryId("choice_request", msg.id as string),
        type: "choice_request",
        role: "system",
        content: msg.prompt as string,
        timestamp: new Date().toISOString(),
        node_id: store.activeNodeId ?? undefined,
        metadata: {
          request_id: msg.id as string,
          prompt: msg.prompt as string,
          options: (msg.options as unknown[]) ?? [],
          questions: Array.isArray(msg.questions) ? msg.questions : [],
          allow_multiple: msg.allow_multiple === true,
          allow_free_text: msg.allow_free_text === true,
          source: typeof msg.source === "string" ? msg.source : "provider_choice",
        },
      });
      break;
    case "provider_status":
      store.setProviderStatus(msg.status as ProviderStatus);
      break;
    case "execution_event":
      {
        const event = msg.event as ExecutionEvent;
        const provider = providerNameForNode(store, event.node_id ?? null, event.agent ?? null);
        store.upsertExecutionEvent(event);
        if (isProviderPromptEvent(event)) {
          store.appendChatEntry({
            id: providerPromptChatEntryId(event),
            type: "execution_event",
            role: entryRoleForNode(store, event.node_id ?? null, "system"),
            content: `${executionEventContent(event, nodeTitleForEvent(store, event))} · ${formatContentSize(event.output.length)}`,
            timestamp: new Date().toISOString(),
            node_id: event.node_id ?? undefined,
            metadata: providerPromptEventMetadata(event, provider),
            content_ref: event.node_id
              ? { kind: "provider_prompt", nodeId: event.node_id }
              : undefined,
            content_size: event.output.length,
            has_full_content: true,
          });
          break;
        }
        store.appendChatEntry({
          id: executionEventChatEntryId(event),
          type: "execution_event",
          role: entryRoleForNode(store, event.node_id ?? null, "system"),
          content: executionEventContent(event, nodeTitleForEvent(store, event)),
          timestamp: new Date().toISOString(),
          node_id: event.node_id ?? undefined,
          metadata: provider ? { ...event, provider } : { ...event },
        });
      }
      break;
    case "timeline_node_created":
      store.addTimelineNode(msg.node as TimelineNode);
      break;
    case "timeline_node_updated":
      store.updateTimelineNode(
        msg.node_id as string,
        msg.status as TimelineNodeStatus,
        msg.summary as string | null | undefined,
        msg.completed_at as string | null | undefined,
      );
      break;
    case "review_complete":
      {
        const findings = Array.isArray(msg.findings) ? msg.findings : [];
        const reviewGate =
          typeof msg.review_gate === "string" ? msg.review_gate : undefined;
        const verdict = {
          verdict: msg.verdict,
          comments: msg.comments,
          summary: msg.summary,
          findings,
          ...(reviewGate ? { review_gate: reviewGate } : {}),
        } as ReviewVerdict;
        store.setNodeVerdict(msg.node_id as string, verdict);
        store.appendChatEntry({
          id: chatEntryId("review_verdict", msg.node_id as string),
          type: "review_verdict",
          role: "reviewer",
          content: msg.summary as string,
          timestamp: new Date().toISOString(),
          node_id: msg.node_id as string,
          metadata: {
            verdict: msg.verdict as string,
            comments: msg.comments as string,
            summary: msg.summary as string,
            round: msg.round as number,
            findings,
            ...(reviewGate ? { review_gate: reviewGate } : {}),
          },
        });
      }
      break;
    case "review_decision_required":
      store.setPendingDecision({
        node_id: msg.node_id as string,
        round: msg.round as number,
        options: msg.options as string[],
      });
      break;
    case "error":
      store.setError(msg.message as string);
      store.appendChatEntry({
        id: chatEntryId("error", msg.message as string),
        type: "error",
        role: "system",
        content: msg.message as string,
        timestamp: new Date().toISOString(),
        metadata: {
          message: msg.message as string,
        },
      });
      break;
    case "protocol_error":
      {
        const code = msg.code as string;
        const message = msg.message as string;
        if (code === "CHOICE_ID_UNMATCHED") {
          const choiceId = choiceIdFromProtocolError(msg.context, message);
          if (choiceId) {
            store.rejectChoiceRequest(choiceId, message);
          }
        }
        store.setProtocolError({
          code,
          message,
        });
      }
      store.appendChatEntry({
        id: chatEntryId("protocol_error", `${msg.code as string}:${msg.message as string}`),
        type: "error",
        role: "system",
        content: `${msg.code as string} · ${msg.message as string}`,
        timestamp: new Date().toISOString(),
        metadata: {
          code: msg.code as string,
          message: msg.message as string,
        },
      });
      break;
    case "provider_locked":
      store.setProviderLocked({
        snapshot: msg.snapshot as ProviderConfigSnapshot,
        locked_at: msg.locked_at as string,
      });
      store.appendChatEntry({
        id: chatEntryId("start_generation", msg.locked_at as string),
        type: "start_generation",
        role: "system",
        content: "开始生成",
        timestamp: msg.locked_at as string,
        metadata: {
          snapshot: msg.snapshot as ProviderConfigSnapshot,
          locked_at: msg.locked_at as string,
        },
      });
      break;
    case "pong":
      break;
  }
}

function handleStreamChunk(store: ReturnType<typeof useWorkspaceStore.getState>, msg: WsServerMessage) {
  const nodeId = resolveStreamEntryNodeId(store, msg.node_id as string | null | undefined);
  const entryId = streamEntryId(nodeId);
  const role = chatEntryRole((msg.role as string | undefined) ?? "author");
  const provider = providerNameForNode(store, nodeId, null);
  if (!store.chatEntries.some((entry) => entry.id === entryId)) {
    store.appendChatEntry({
      id: entryId,
      type: "provider_stream",
      role,
      content: "",
      timestamp: new Date().toISOString(),
      node_id: nodeId === "global" ? undefined : nodeId,
      metadata: provider ? { provider } : undefined,
    });
  }
  store.updateStreamingEntry(entryId, msg.content as string);
}

function chatEntryRole(role: string): ChatEntryRole {
  return role === "reviewer" ? "reviewer" : "author";
}

function resolveStreamEntryId(
  store: ReturnType<typeof useWorkspaceStore.getState>,
  nodeId?: string | null,
) {
  if (nodeId) {
    return `${nodeId}:stream-active`;
  }
  return streamEntryId(resolveStreamEntryNodeId(store, nodeId));
}

function resolveStreamEntryNodeId(
  store: ReturnType<typeof useWorkspaceStore.getState>,
  nodeId?: string | null,
) {
  return nodeId ?? store.activeNodeId ?? "global";
}

function streamEntryId(nodeId: string | null | undefined) {
  return `${nodeId ?? "global"}:stream`;
}

function chatEntryId(kind: string, suffix: string) {
  return `${kind}:${suffix}`;
}

function workItemPlanArtifactFromMessage(msg: WsServerMessage): WorkItemPlanArtifactPayload | null {
  if (msg.outline_candidate) {
    return {
      type: "outline_candidate",
      payload: msg.outline_candidate as WorkItemPlanOutlineCandidatePayload,
    };
  }
  if (msg.context_blocker) {
    return {
      type: "context_blocker",
      payload: msg.context_blocker as WorkItemPlanContextBlockerPayload,
    };
  }
  if (msg.draft_candidate) {
    return {
      type: "draft_candidate",
      payload: msg.draft_candidate as WorkItemDraftCandidatePayload,
    };
  }
  if (msg.batch_state) {
    return {
      type: "batch_state",
      payload: msg.batch_state as WorkItemBatchStatePayload,
    };
  }
  if (msg.compile_report) {
    return {
      type: "compile_report",
      payload: msg.compile_report as WorkItemPlanCompileReportPayload,
    };
  }
  return null;
}

function permissionRequestContent(request: { tool_name: string; description: string }) {
  return request.description ? `${request.tool_name} · ${request.description}` : request.tool_name;
}

function executionEventContent(event: ExecutionEvent, nodeTitle?: string | null) {
  const command = event.kind === "command" ? event.command?.trim() : "";
  if (command) {
    return command;
  }
  if (event.title === "Provider Prompt" && typeof event.output === "string" && nodeTitle) {
    return `${nodeTitle} · Provider Prompt`;
  }
  return event.detail ? `${event.title} · ${event.detail}` : event.title;
}

function isProviderPromptEvent(
  event: Pick<ExecutionEvent, "title" | "output">,
): event is Pick<ExecutionEvent, "title" | "output"> & { output: string } {
  return event.title === "Provider Prompt" && typeof event.output === "string";
}

function providerPromptChatEntryId(event: ExecutionEvent) {
  return event.node_id
    ? chatEntryId(event.node_id, "provider-prompt")
    : chatEntryId("execution_event", event.event_id);
}

function executionEventChatEntryId(event: ExecutionEvent) {
  return event.node_id
    ? chatEntryId(event.node_id, `execution-${event.event_id}`)
    : chatEntryId("execution_event", event.event_id);
}

function providerPromptEventMetadata(event: ExecutionEvent, provider?: string | null) {
  return {
    event_id: event.event_id,
    node_id: event.node_id ?? null,
    agent: event.agent ?? null,
    kind: event.kind,
    status: event.status,
    title: event.title,
    detail: event.detail ?? null,
    command: event.command ?? null,
    cwd: event.cwd ?? null,
    exit_code: event.exit_code ?? null,
    ...(provider ? { provider } : {}),
  };
}

function formatContentSize(chars: number) {
  if (chars < 1024) {
    return `${chars} 字符`;
  }
  return `约 ${Math.ceil(chars / 1024)}KB`;
}

function nodeTitleForEvent(
  store: ReturnType<typeof useWorkspaceStore.getState>,
  event: ExecutionEvent,
) {
  return event.node_id
    ? store.timelineNodes.find((candidate) => candidate.node_id === event.node_id)?.title ?? null
    : null;
}

function entryRoleForNode(
  store: ReturnType<typeof useWorkspaceStore.getState>,
  nodeId: string | null | undefined,
  fallback: ChatEntryRole,
) {
  const node = nodeId
    ? store.timelineNodes.find((candidate) => candidate.node_id === nodeId)
    : null;
  const nodeRole = chatRoleForTimelineNode(node ?? undefined);
  if (nodeRole) {
    return nodeRole;
  }
  const detail = nodeId ? store.nodeDetails[nodeId] : null;
  if (detail?.agent_role === "reviewer") {
    return "reviewer";
  }
  if (detail?.agent_role === "author") {
    return "author";
  }
  return fallback;
}

function providerNameForNode(
  store: ReturnType<typeof useWorkspaceStore.getState>,
  nodeId: string | null | undefined,
  fallback: string | null,
) {
  const node = nodeId
    ? store.timelineNodes.find((candidate) => candidate.node_id === nodeId)
    : null;
  const detail = nodeId ? store.nodeDetails[nodeId] : null;
  const provider = node?.agent ?? detail?.provider?.name ?? fallback;
  return typeof provider === "string" && provider.length > 0 ? provider : null;
}

export function providerName(value: string): WorkspaceProviderName | null {
  if (value === "claude_code" || value === "codex" || value === "fake") {
    return value;
  }
  return null;
}

function choiceIdFromProtocolError(context: unknown, message: string) {
  if (isRecord(context) && typeof context.choice_id === "string") {
    return context.choice_id;
  }
  return message.match(/^ChoiceResponse id=(.+) not found in pending$/)?.[1] ?? null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function wsReadyStateName(socket: WebSocket | null) {
  switch (socket?.readyState) {
    case WebSocket.CONNECTING:
      return "connecting";
    case WebSocket.OPEN:
      return "open";
    case WebSocket.CLOSING:
      return "closing";
    case WebSocket.CLOSED:
      return "closed";
    default:
      return "missing";
  }
}

function gatePromptEntryForState(state: ReturnType<typeof useWorkspaceStore.getState>): ChatEntry | null {
  if (state.stage !== "human_confirm") {
    return null;
  }

  const gatePromptNode =
    [...state.timelineNodes].reverse().find((node) => node.node_type === "human_confirm") ??
    state.timelineNodes.at(-1);
  const latestReview = state.chatEntries.filter((entry) => entry.type === "review_verdict").at(-1);
  const summary = latestReview?.metadata?.summary?.toString() ?? "";
  const verdict = latestReview?.metadata?.verdict?.toString() ?? "";
  const comments = latestReview?.metadata?.comments?.toString() ?? "";
  const findings = Array.isArray(latestReview?.metadata?.findings)
    ? latestReview.metadata.findings
    : [];
  const reviewGate = latestReview?.metadata?.review_gate?.toString() ?? "";
  const metadata = {
    ...(summary ? { summary } : {}),
    ...(verdict ? { verdict } : {}),
    ...(comments ? { comments } : {}),
    ...(findings.length > 0 ? { findings } : {}),
    ...(reviewGate ? { review_gate: reviewGate } : {}),
  };
  return {
    id: `${gatePromptNode?.node_id ?? "human_confirm"}:gate-prompt`,
    type: "gate_prompt",
    role: "system",
    content: verdict === "needs_human" ? "需要人工确认" : "等待人工确认",
    timestamp: gatePromptNode?.completed_at ?? gatePromptNode?.started_at ?? new Date().toISOString(),
    node_id: gatePromptNode?.node_id,
    metadata: Object.keys(metadata).length > 0 ? metadata : undefined,
  };
}
