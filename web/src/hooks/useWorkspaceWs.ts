import { useCallback, useEffect, useRef, useState } from "react";
import type {
  AuthorDecision,
  HumanConfirmDecision,
  ProviderConfigSnapshot,
  RevisionPath,
  WorkspaceProviderName,
  WorkItemBatchDecision,
  WorkItemBatchStatePayload,
  WorkItemDraftCandidatePayload,
  WorkItemDraftDecision,
  WorkItemGenerationMode,
  WorkItemPlanArtifactPayload,
  WorkItemPlanCandidateDto,
  WorkItemPlanCompileRecoveryAction,
  WorkItemPlanCompileReportPayload,
  WorkItemPlanContextBlockerPayload,
  WorkItemPlanOutlineCandidatePayload,
  WsInMessage,
} from "../api/types";
import type { ChatEntry, ChatEntryRole } from "../state/chat-entries";
import { useWorkspaceWsReconnect } from "./useWorkspaceWsReconnect";
import {
  useWorkspaceStore,
  type ExecutionEvent,
  type ProviderStatus,
  type ReviewVerdict,
  type TimelineNode,
  type TimelineNodeStatus,
} from "../state/workspace-ws-store";

interface WsServerMessage {
  type: string;
  [key: string]: unknown;
}

type WorkspaceWsSendMessage =
  | WsInMessage
  | { type: "provider_select"; role: string; provider: string };

const PING_INTERVAL_MS = 25_000;
const SERVER_SILENCE_TIMEOUT_MS = 60_000;
const STALE_SOCKET_CLOSE_CODE = 4000;
const SERVER_SILENCE_CHECK_INTERVAL_MS = 15_000;
const CONNECT_TIMEOUT_MS = 5_000;
const STREAM_FLUSH_INTERVAL_MS = 80;
const ACTIVE_PROVIDER_STAGES = new Set(["running", "cross_review", "revision"]);

export function useWorkspaceWs(sessionId: string | null) {
  const wsRef = useRef<WebSocket | null>(null);
  const connectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const streamFlushTimeoutsRef = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const invalidatedPreStageNodeIdsRef = useRef<Set<string>>(new Set());
  const lastMessageAtRef = useRef(Date.now());
  const [closeCode, setCloseCode] = useState<number | undefined>();
  const connectionStatus = useWorkspaceStore((state) => state.connectionStatus);

  const clearConnectTimeout = useCallback(() => {
    if (connectTimeoutRef.current) {
      clearTimeout(connectTimeoutRef.current);
      connectTimeoutRef.current = null;
    }
  }, []);

  const clearStreamFlushTimeouts = useCallback(() => {
    for (const timeout of Object.values(streamFlushTimeoutsRef.current)) {
      clearTimeout(timeout);
    }
    streamFlushTimeoutsRef.current = {};
  }, []);

  const clearPendingStreams = useCallback(() => {
    clearStreamFlushTimeouts();
    useWorkspaceStore.getState().clearAllStreamBuffers();
  }, [clearStreamFlushTimeouts]);

  const scheduleFlush = useCallback((nodeId: string) => {
    if (streamFlushTimeoutsRef.current[nodeId]) {
      return;
    }
    streamFlushTimeoutsRef.current[nodeId] = setTimeout(() => {
      delete streamFlushTimeoutsRef.current[nodeId];
      useWorkspaceStore.getState().flushBufferedStream(nodeId);
    }, STREAM_FLUSH_INTERVAL_MS);
  }, []);

  const connect = useCallback(() => {
    if (!sessionId) return;

    const current = wsRef.current;
    if (
      current &&
      (current.readyState === WebSocket.CONNECTING || current.readyState === WebSocket.OPEN)
    ) {
      return;
    }

    setCloseCode(undefined);
    useWorkspaceStore.getState().setConnectionStatus("connecting");

    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const url = `${protocol}//${window.location.host}/api/workspace-sessions/${sessionId}/ws`;
    const ws = new WebSocket(url);
    wsRef.current = ws;
    clearConnectTimeout();
    connectTimeoutRef.current = setTimeout(() => {
      if (wsRef.current !== ws) return;
      wsRef.current = null;
      setCloseCode(1006);
      const store = useWorkspaceStore.getState();
      store.setConnectionStatus("disconnected");
      store.setError("WebSocket 连接超时");
      ws.close();
    }, CONNECT_TIMEOUT_MS);

    ws.onopen = () => {
      if (wsRef.current !== ws) return;
      clearConnectTimeout();
      const store = useWorkspaceStore.getState();
      lastMessageAtRef.current = Date.now();
      store.setConnectionStatus("connected");
      store.setError(null);
      setCloseCode(undefined);
      ws.send(
        JSON.stringify({
          type: "hello",
          session_id: sessionId,
          last_seen_node_id: store.activeNodeId ?? store.timelineNodes.at(-1)?.node_id ?? null,
        }),
      );
    };

    ws.onclose = (event) => {
      if (wsRef.current !== ws) return;
      clearConnectTimeout();
      clearPendingStreams();
      wsRef.current = null;
      setCloseCode(event.code);
      useWorkspaceStore.getState().setConnectionStatus("disconnected");
    };

    ws.onerror = () => {
      if (wsRef.current !== ws) return;
      clearConnectTimeout();
      clearPendingStreams();
      const store = useWorkspaceStore.getState();
      wsRef.current = null;
      setCloseCode(1006);
      store.setConnectionStatus("disconnected");
      store.setError("WebSocket 连接失败");
    };

    ws.onmessage = (event) => {
      lastMessageAtRef.current = Date.now();
      try {
        const msg = JSON.parse(event.data) as WsServerMessage;
        handleMessage(msg);
      } catch {
        // ignore malformed messages
      }
    };
  }, [clearConnectTimeout, clearPendingStreams, sessionId]);

  const {
    isReconnecting,
    attemptCount: reconnectAttemptCount,
    retryNow,
    reset: resetReconnect,
  } = useWorkspaceWsReconnect({
    enabled:
      Boolean(sessionId) &&
      connectionStatus === "disconnected" &&
      closeCode !== undefined &&
      closeCode !== 1000,
    closeCode,
    onReconnect: connect,
  });

  useEffect(() => {
    if (!sessionId) {
      clearPendingStreams();
      useWorkspaceStore.getState().reset();
      return;
    }

    connect();

    return () => {
      clearConnectTimeout();
      clearPendingStreams();
      const ws = wsRef.current;
      wsRef.current = null;
      ws?.close(1000);
    };
  }, [clearConnectTimeout, clearPendingStreams, connect, sessionId]);

  useEffect(() => {
    if (connectionStatus === "connected") {
      resetReconnect();
    }
  }, [connectionStatus, resetReconnect]);

  function handleMessage(msg: WsServerMessage) {
    const store = useWorkspaceStore.getState();
    switch (msg.type) {
      case "session_state":
        invalidatedPreStageNodeIdsRef.current.clear();
        store.setSessionState(msg as never);
        store.rebuildChatEntries();
        break;
      case "stream_chunk":
        {
          const nodeId = msg.node_id as string | null | undefined;
          if (
            typeof nodeId === "string" &&
            invalidatedPreStageNodeIdsRef.current.has(nodeId)
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
            !invalidatedPreStageNodeIdsRef.current.has(nodeId);
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
          const timeout = streamFlushTimeoutsRef.current[nodeId];
          if (timeout) {
            clearTimeout(timeout);
            delete streamFlushTimeoutsRef.current[nodeId];
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
          const nextStage = msg.stage as string;
          if (!ACTIVE_PROVIDER_STAGES.has(nextStage) && store.activeNodeId) {
            invalidatedPreStageNodeIdsRef.current.add(store.activeNodeId);
          }
          store.setStage(nextStage);
        }
        store.appendChatEntry({
          id: chatEntryId("stage_change", `${msg.stage as string}:${store.chatEntries.length}`),
          type: "stage_change",
          role: "system",
          content: `阶段变更 -> ${msg.stage as string}`,
          timestamp: new Date().toISOString(),
        });
        if (msg.stage === "human_confirm") {
          const gatePrompt = gatePromptEntryForState(useWorkspaceStore.getState());
          if (gatePrompt) {
            store.appendChatEntry(gatePrompt);
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
              store.appendChatEntry({
                id: chatEntryId("artifact_update", `${workItemPlanArtifact.type}:${String(version)}`),
                type: "artifact_update",
                role: "system",
                content: `Work Item Plan staged artifact 已更新 -> v${version}`,
                timestamp: new Date().toISOString(),
                metadata: {
                  version,
                  artifact_type: workItemPlanArtifact.type,
                },
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
            id: chatEntryId("execution_event", event.event_id),
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

  const sendJson = useCallback((message: WorkspaceWsSendMessage) => {
    const ws = wsRef.current;
    if (ws?.readyState !== WebSocket.OPEN) {
      return false;
    }
    ws.send(JSON.stringify(message));
    return true;
  }, []);

  const sendContextNote = useCallback(
    (content: string) => {
      if (sendJson({ type: "context_note", content })) {
        useWorkspaceStore.getState().setError(null);
      }
    },
    [sendJson],
  );

  const sendStartGeneration = useCallback(
    (providerConfig: ProviderConfigSnapshot, reviewerEnabled: boolean) => {
      if (
        sendJson({
          type: "start_generation",
          provider_config: providerConfig,
          reviewer_enabled: reviewerEnabled,
        })
      ) {
        const store = useWorkspaceStore.getState();
        store.setError(null);
        store.clearExecutionEvents();
        store.setProviderStatus("running");
      }
    },
    [sendJson],
  );

  const sendHello = useCallback(
    (targetSessionId: string, lastSeenNodeId?: string | null) => {
      sendJson({
        type: "hello",
        session_id: targetSessionId,
        last_seen_node_id: lastSeenNodeId ?? null,
      });
    },
    [sendJson],
  );

  const sendPing = useCallback(() => {
    sendJson({ type: "ping" });
  }, [sendJson]);

  useEffect(() => {
    if (connectionStatus !== "connected") return;

    const interval = window.setInterval(() => {
      sendPing();
    }, PING_INTERVAL_MS);

    return () => window.clearInterval(interval);
  }, [connectionStatus, sendPing]);

  useEffect(() => {
    if (connectionStatus !== "connected") return;

    const interval = window.setInterval(() => {
      const ws = wsRef.current;
      const stage = useWorkspaceStore.getState().stage;
      if (
        ws?.readyState === WebSocket.OPEN &&
        !ACTIVE_PROVIDER_STAGES.has(stage) &&
        Date.now() - lastMessageAtRef.current >= SERVER_SILENCE_TIMEOUT_MS
      ) {
        ws.close(STALE_SOCKET_CLOSE_CODE);
      }
    }, SERVER_SILENCE_CHECK_INTERVAL_MS);

    return () => window.clearInterval(interval);
  }, [connectionStatus]);

  const sendSelectRevisionPath = useCallback(
    (path: RevisionPath, extraContext?: string) => {
      const trimmedContext = extraContext?.trim();
      sendJson({
        type: "select_revision_path",
        path,
        extra_context: trimmedContext ? trimmedContext : null,
      });
    },
    [sendJson],
  );

  const sendHumanConfirm = useCallback(
    (decision: HumanConfirmDecision, payload?: unknown) => {
      if (sendJson({ type: "human_confirm", decision, payload: payload ?? null })) {
        useWorkspaceStore.getState().resolveGateEntry(decision);
      }
    },
    [sendJson],
  );

  const sendAuthorDecision = useCallback(
    (decision: AuthorDecision) => {
      sendJson({ type: "author_decision", decision });
    },
    [sendJson],
  );

  const sendRequestRevision = useCallback(
    (feedback?: string) => {
      const trimmedFeedback = feedback?.trim();
      sendJson({
        type: "request_revision",
        feedback: {
          feedback_types: ["revision"],
          description: trimmedFeedback ?? "",
        },
      });
    },
    [sendJson],
  );

  const sendRevertWorkItem = useCallback(
    (workItemId: string, feedback?: string, clear = false) => {
      sendJson({
        type: "revert_work_item",
        work_item_id: workItemId,
        feedback: feedback?.trim() ?? null,
        clear,
      });
    },
    [sendJson],
  );

  const sendSelectWorkItemGenerationMode = useCallback(
    (mode: WorkItemGenerationMode) => {
      sendJson({ type: "select_work_item_generation_mode", mode });
    },
    [sendJson],
  );

  const sendRequestOutlineRevision = useCallback(
    (feedback?: string) => {
      const trimmedFeedback = feedback?.trim();
      sendJson({
        type: "request_outline_revision",
        feedback: trimmedFeedback ? trimmedFeedback : null,
      });
    },
    [sendJson],
  );

  const sendWorkItemDraftDecision = useCallback(
    (outlineId: string, decision: WorkItemDraftDecision, feedback?: string) => {
      const trimmedFeedback = feedback?.trim();
      sendJson({
        type: "work_item_draft_decision",
        outline_id: outlineId,
        decision,
        feedback: trimmedFeedback ? trimmedFeedback : null,
      });
    },
    [sendJson],
  );

  const sendWorkItemBatchDecision = useCallback(
    (
      decision: WorkItemBatchDecision,
      feedback?: string,
      firstAffectedOutlineId?: string,
    ) => {
      const trimmedFeedback = feedback?.trim();
      const trimmedOutlineId = firstAffectedOutlineId?.trim();
      sendJson({
        type: "work_item_batch_decision",
        decision,
        feedback: trimmedFeedback ? trimmedFeedback : null,
        first_affected_outline_id: trimmedOutlineId ? trimmedOutlineId : null,
      });
    },
    [sendJson],
  );

  const sendWorkItemPlanCompileRecoveryAction = useCallback(
    (action: WorkItemPlanCompileRecoveryAction, reason?: string) => {
      const trimmedReason = reason?.trim();
      sendJson({
        type: "work_item_plan_compile_recovery_action",
        action,
        reason: trimmedReason ? trimmedReason : null,
      });
    },
    [sendJson],
  );

  const sendMessage = useCallback(
    (content: string) => {
      console.warn("sendMessage is deprecated, use sendContextNote or sendStartGeneration");
      sendContextNote(content);
    },
    [sendContextNote],
  );

  const startGeneration = useCallback(() => {
    console.warn("startGeneration() without args is deprecated");
  }, []);

  const rollback = useCallback(
    (checkpointId: string) => {
      sendJson({ type: "rollback", checkpoint_id: checkpointId });
    },
    [sendJson],
  );

  const confirm = useCallback(() => {
    sendJson({ type: "confirm" });
  }, [sendJson]);

  const abort = useCallback(() => {
    sendJson({ type: "abort" });
  }, [sendJson]);

  const selectProvider = useCallback(
    (role: string, provider: string) => {
      if (sendJson({ type: "provider_select", role, provider })) {
        const validRole = role === "author" || role === "reviewer";
        const validProvider = providerName(provider);
        if (validRole && validProvider) {
          useWorkspaceStore.getState().setProviderSelection(role, validProvider);
        }
      }
    },
    [sendJson],
  );

  const sendReviewDecision = useCallback((decision: string, extraContext?: string) => {
    const trimmedContext = extraContext?.trim();
    sendJson({
      type: "review_decision_response",
      decision,
      extra_context: trimmedContext ? trimmedContext : null,
    });
  }, [sendJson]);

  const respondPermission = useCallback(
    (id: string, approved: boolean, reason?: string) => {
      const trimmedReason = reason?.trim();
      if (
        sendJson({
          type: "permission_response",
          id,
          approved,
          reason: trimmedReason ? trimmedReason : null,
        })
      ) {
        console.info("[permission] sending response", { id, approved });
        useWorkspaceStore.getState().resolvePermissionRequest(id, approved);
      }
    },
    [sendJson],
  );

  const sendChoiceResponse = useCallback(
    (id: string, selectedOptionIds: string[], freeText?: string | null) => {
      const trimmedText = freeText?.trim();
      const store = useWorkspaceStore.getState();
      const choiceEntry = store.chatEntries.find(
        (entry) => entry.type === "choice_request" && entry.metadata?.request_id === id,
      );
      console.info("[aria-choice-diag] frontend choice_response send attempt", {
        id,
        selected_option_ids: selectedOptionIds,
        free_text_present: Boolean(trimmedText),
        source: choiceEntry?.metadata?.source ?? null,
        node_id: choiceEntry?.node_id ?? null,
        connection_status: store.connectionStatus,
        ws_ready_state: wsReadyStateName(wsRef.current),
      });
      const sent = sendJson({
        type: "choice_response",
        id,
        selected_option_ids: selectedOptionIds,
        free_text: trimmedText ? trimmedText : null,
      });
      console.info("[aria-choice-diag] frontend choice_response send result", {
        id,
        sent,
      });
      if (sent) {
        useWorkspaceStore
          .getState()
          .resolveChoiceRequest(id, selectedOptionIds, trimmedText ? trimmedText : null);
      }
    },
    [sendJson],
  );

  const sendProviderSelect = selectProvider;
  const sendPermissionResponse = respondPermission;

  return {
    sendMessage,
    sendContextNote,
    sendStartGeneration,
    sendSelectRevisionPath,
    sendAuthorDecision,
    sendRequestRevision,
    sendRevertWorkItem,
    sendSelectWorkItemGenerationMode,
    sendRequestOutlineRevision,
    sendWorkItemDraftDecision,
    sendWorkItemBatchDecision,
    sendWorkItemPlanCompileRecoveryAction,
    sendHumanConfirm,
    sendHello,
    sendPing,
    startGeneration,
    rollback,
    confirm,
    abort,
    selectProvider,
    sendProviderSelect,
    sendReviewDecision,
    respondPermission,
    sendPermissionResponse,
    sendChoiceResponse,
    connectionStatus,
    isReconnecting,
    reconnectAttemptCount,
    retryNow,
  };
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
  if (node?.node_type === "reviewer_run") {
    return "reviewer";
  }
  if (node?.node_type === "author_run") {
    return "author";
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

function providerName(value: string): WorkspaceProviderName | null {
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

function wsReadyStateName(socket: WebSocket | null) {
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
