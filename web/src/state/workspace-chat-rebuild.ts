import type {
  ChatEntry,
  ChatEntryRole,
  ChoiceResponsePayload,
} from "./chat-entries";
import { workItemPlanArtifactUpdateSummary } from "./work-item-plan-artifact-summary";
import { chatRoleForTimelineNode } from "./workspace-ws-store-helpers";
import type {
  ExecutionEvent,
  NodeDetailSummary,
  TimelineNode,
  TimelineNodeDetail,
  TimelineNodeType,
  WorkspaceWsState,
  WsMessage,
} from "./workspace-ws-store-types";

export function buildChatEntries(state: WorkspaceWsState): ChatEntry[] {
  const entries: ChatEntry[] = [];
  const retriedNodeIds = retriedTimelineNodeIds(state.timelineNodes);

  for (const message of state.messages) {
    if (!isPreparedWorkspaceContextMessage(message)) {
      continue;
    }

    entries.push({
      id: `prepared-context:${message.id}`,
      type: "context_note",
      role: "user",
      content: message.content,
      timestamp: message.created_at,
      metadata: { prepared: true },
    });
  }

  for (const node of state.timelineNodes) {
    if (retriedNodeIds.has(node.node_id)) {
      continue;
    }

    const detail = state.nodeDetails[node.node_id];
    if (!detail) {
      continue;
    }

    if (node.node_type === "context_note") {
      const content = textFromSources([
        detail.streaming_content,
        node.summary,
        detail.messages.map((message) => message.content).join("\n"),
      ]);
      if (content) {
        entries.push({
          id: chatEntryId(node.node_id, "context"),
          type: "context_note",
          role: "user",
          content,
          timestamp: detail.started_at || node.started_at,
          node_id: node.node_id,
        });
      }
      continue;
    }

    const role = chatRoleForNode(node, state.workspaceType, detail);
    if (!role) {
      const marker = timelineAnchorEntry(node, detail);
      if (marker) {
        entries.push(marker);
      }
      continue;
    }

    const summary = state.nodeSummaries[node.node_id];
    const hasPersistedDetailContent = Boolean(
      detail.prompt?.trim() ||
        detail.streaming_content.trim() ||
        detail.messages.length > 0 ||
        detail.execution_events.length > 0,
    );
    if (!hasPersistedDetailContent && summary) {
      entries.push(...providerSummaryEntries(node, summary, role));
      continue;
    }

    const latestProviderPrompt = latestProviderPromptEvent(detail.execution_events);
    let renderedProviderPromptEntry = false;
    const prompt = detail.prompt?.trim();
    if (prompt && !latestProviderPrompt) {
      const provider = providerNameForNode(node, detail);
      entries.push({
        id: chatEntryId(node.node_id, "provider-prompt"),
        type: "execution_event",
        role,
        content: `${providerPromptContent(node.title)} · ${formatContentSize(prompt.length)}`,
        timestamp: detail.started_at || node.started_at,
        node_id: node.node_id,
        metadata: {
          event_id: `${node.node_id}_prompt`,
          node_id: node.node_id,
          agent: provider,
          kind: "output",
          status: "started",
          title: "Provider Prompt",
          detail: "发送给 Workspace provider 的完整提示词",
          command: null,
          cwd: null,
          exit_code: null,
          ...providerEntryMetadata(node, provider),
        },
        content_ref: { kind: "provider_prompt", nodeId: node.node_id },
        content_size: prompt.length,
        has_full_content: true,
      });
      renderedProviderPromptEntry = true;
    } else if (!latestProviderPrompt && summary && summary.prompt_size > 0) {
      entries.push(providerPromptSummaryEntry(node, summary, role));
      renderedProviderPromptEntry = true;
    }

    const streamContent = textFromSources([
      detail.streaming_content,
      detail.messages.map((message) => message.content).join("\n"),
    ]);
    if (streamContent) {
      const provider = providerNameForNode(node, detail);
      entries.push({
        id: chatEntryId(node.node_id, "stream"),
        type: "provider_stream",
        role,
        content: streamContent,
        timestamp: detail.started_at || node.started_at,
        node_id: node.node_id,
        metadata: providerEntryMetadata(node, provider),
      });
    }

    for (const event of detail.execution_events) {
      const timestamp = detail.started_at || node.started_at;
      const provider = providerNameForNode(node, detail, event);
      if (isProviderPromptEvent(event)) {
        if (event !== latestProviderPrompt) {
          continue;
        }
        entries.push({
          id: chatEntryId(node.node_id, "provider-prompt"),
          type: "execution_event",
          role,
          content: `${executionEventContent(event, node.title)} · ${formatContentSize(event.output.length)}`,
          timestamp,
          node_id: node.node_id,
          metadata: providerPromptEventMetadata(event, provider, node),
          content_ref: { kind: "provider_prompt", nodeId: node.node_id },
          content_size: event.output.length,
          has_full_content: true,
        });
        renderedProviderPromptEntry = true;
        continue;
      }
      if (isProviderPromptTitle(event) && renderedProviderPromptEntry) {
        continue;
      }
      entries.push({
        id: chatEntryId(node.node_id, `execution-${event.event_id}`),
        type: "execution_event",
        role,
        content: executionEventContent(event, node.title),
        timestamp,
        node_id: node.node_id,
        metadata: provider ? { ...event, provider } : { ...event },
        content_ref: { kind: "execution_output", nodeId: node.node_id, eventId: event.event_id },
        content_size: event.output?.length,
        has_full_content: typeof event.output === "string",
      });
    }

    for (const permission of detail.permission_events) {
      const request = permission.request;
      const requestToolName = stringField(request, "tool_name") ?? "权限请求";
      const requestDescription = stringField(request, "description") ?? "";
      const requestRiskLevel = stringField(request, "risk_level") ?? null;
      const response = permission.response;

      entries.push({
        id: chatEntryId(node.node_id, `permission-request-${permission.request_id}`),
        type: "permission_request",
        role: "system",
        content: requestDescription
          ? `${requestToolName} · ${requestDescription}`
          : requestToolName,
        timestamp: permission.ts,
        node_id: node.node_id,
        metadata: {
          request_id: permission.request_id,
          request,
          response,
          risk_level: requestRiskLevel,
          ts: permission.ts,
        },
      });

      if (response) {
        entries.push({
          id: chatEntryId(node.node_id, `permission-response-${permission.request_id}`),
          type: "permission_response",
          role: "user",
          content: permissionResponseLabel(requestToolName, response),
          timestamp: permission.ts,
          node_id: node.node_id,
          metadata: {
            request_id: permission.request_id,
            request,
            response,
            ts: permission.ts,
          },
        });
      }
    }

    const artifactVersions = state.artifactVersions
      .filter((artifact) => artifact.source_node_id === node.node_id)
      .sort((left, right) => left.version - right.version);
    for (const artifact of artifactVersions) {
      const typedArtifact = state.workItemPlanArtifactVersions.find(
        (candidate) => candidate.version === artifact.version,
      )?.artifact;
      const summary = typedArtifact
        ? workItemPlanArtifactUpdateSummary(typedArtifact, artifact.version)
        : null;
      const artifactMetadata = {
        version: artifact.version,
        generated_by: artifact.generated_by,
        reviewed_by: artifact.reviewed_by ?? null,
        review_verdict: artifact.review_verdict ?? null,
        confirmed_by: artifact.confirmed_by ?? null,
        source_node_id: artifact.source_node_id,
        ...(summary?.metadata ?? {}),
      };
      entries.push({
        id: chatEntryId(node.node_id, `artifact-${artifact.version}`),
        type: "artifact_update",
        role: "system",
        content: summary?.content ?? `产物已更新 -> v${artifact.version}`,
        timestamp: artifact.created_at,
        node_id: node.node_id,
        metadata: artifactMetadata,
        content_ref: {
          kind: "artifact_version",
          version: artifact.version,
          sourceNodeId: artifact.source_node_id,
        },
        content_size: typeof artifact.markdown === "string" ? artifact.markdown.length : undefined,
        has_full_content: typeof artifact.markdown === "string",
      });
    }

    if (detail.verdict) {
      const verdictSummary = getStringField(detail.verdict, "summary") ?? "审核结论";
      const verdictValue = getStringField(detail.verdict, "verdict") ?? "revise";
      const verdictComments = getStringField(detail.verdict, "comments") ?? "";
      const verdictFindings = getArrayField(detail.verdict, "findings");
      const reviewGate = getStringField(detail.verdict, "review_gate") ?? "user_confirm_allowed";
      entries.push({
        id: chatEntryId(node.node_id, "review-verdict"),
        type: "review_verdict",
        role: "reviewer",
        content: verdictSummary,
        timestamp: detail.ended_at ?? detail.started_at,
        node_id: node.node_id,
        metadata: {
          verdict: verdictValue,
          comments: verdictComments,
          summary: verdictSummary,
          findings: verdictFindings,
          review_gate: reviewGate,
        },
      });
    }
  }

  const gatePrompt = buildGatePromptEntry(state, entries);
  if (gatePrompt) {
    entries.push(gatePrompt);
  }

  return entries;
}

function providerSummaryEntries(
  node: TimelineNode,
  summary: NodeDetailSummary,
  role: ChatEntryRole,
): ChatEntry[] {
  const entries: ChatEntry[] = [];
  const provider = summary.provider_name ?? node.agent ?? null;
  const timestamp = summary.started_at || node.started_at;
  const streamPreview = summary.stream_preview?.trim();

  if (streamPreview) {
    entries.push({
      id: chatEntryId(node.node_id, "stream-summary"),
      type: "provider_stream",
      role,
      content: streamPreview,
      timestamp,
      node_id: node.node_id,
      metadata: providerEntryMetadata(node, provider),
    });
  }

  if (summary.prompt_size > 0) {
    entries.push(providerPromptSummaryEntry(node, summary, role));
  }

  if (summary.has_large_outputs || summary.execution_event_count > 0) {
    entries.push({
      id: chatEntryId(node.node_id, "execution-output-summary"),
      type: "execution_event",
      role,
      content: `Execution Output · ${summary.has_large_outputs ? "按需加载" : "摘要"}`,
      timestamp,
      node_id: node.node_id,
      metadata: {
        event_id: `${node.node_id}_output`,
        node_id: node.node_id,
        agent: provider,
        kind: "output",
        status: summary.status,
        title: "Execution Output",
        detail: "Provider execution output 按需加载",
        command: null,
        cwd: null,
        exit_code: null,
        ...providerEntryMetadata(node, provider),
      },
      content_ref: {
        kind: "execution_output",
        nodeId: node.node_id,
        eventId: `${node.node_id}_output`,
      },
      has_full_content: true,
    });
  }

  return entries;
}

function providerPromptSummaryEntry(
  node: TimelineNode,
  summary: NodeDetailSummary,
  role: ChatEntryRole,
): ChatEntry {
  const provider = summary.provider_name ?? node.agent ?? null;
  const timestamp = summary.started_at || node.started_at;
  return {
    id: chatEntryId(node.node_id, "provider-prompt"),
    type: "execution_event",
    role,
    content: `${providerPromptContent(node.title)} · ${formatContentSize(summary.prompt_size)}`,
    timestamp,
    node_id: node.node_id,
    metadata: {
      event_id: `${node.node_id}_prompt`,
      node_id: node.node_id,
      agent: provider,
      kind: "output",
      status: summary.status,
      title: "Provider Prompt",
      detail: "发送给 Workspace provider 的完整提示词",
      command: null,
      cwd: null,
      exit_code: null,
      ...providerEntryMetadata(node, provider),
    },
    content_ref: { kind: "provider_prompt", nodeId: node.node_id },
    content_size: summary.prompt_size,
    has_full_content: true,
  };
}

function chatRoleForNode(
  node: TimelineNode,
  _workspaceType: string | null,
  _detail: TimelineNodeDetail,
): ChatEntryRole | null {
  return chatRoleForTimelineNode(node);
}

function timelineAnchorEntry(node: TimelineNode, detail: TimelineNodeDetail): ChatEntry | null {
  if (!shouldRenderTimelineAnchor(node)) {
    return null;
  }

  return {
    id: chatEntryId(node.node_id, "timeline-anchor"),
    type: node.node_type === "start_generation" ? "start_generation" : "stage_change",
    role: "system",
    content: timelineAnchorContent(node),
    timestamp: detail.started_at || node.started_at,
    node_id: node.node_id,
    metadata: {
      node_type: node.node_type,
      status: node.status,
      stage: node.stage,
      summary: node.summary ?? null,
      snapshot: node.provider_config_snapshot,
    },
  };
}

function shouldRenderTimelineAnchor(node: TimelineNode) {
  return [
    "start_generation",
    "author_confirm",
    "review_decision",
    "completed",
    "aborted_by_disconnect",
    "protocol_error",
  ].includes(node.node_type);
}

function timelineAnchorContent(node: TimelineNode) {
  const summary = node.summary?.trim();
  return summary ? `${node.title} · ${summary}` : node.title;
}

function buildGatePromptEntry(
  state: WorkspaceWsState,
  entries = state.chatEntries,
): ChatEntry | null {
  if (state.stage !== "human_confirm") {
    return null;
  }

  const gatePromptNode =
    findLatestNodeOfType(state.timelineNodes, "human_confirm") ?? state.timelineNodes.at(-1);
  const latestReview = entries.filter((entry) => entry.type === "review_verdict").at(-1);
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
  const fallbackContent = verdict === "needs_human" ? "需要人工确认" : "等待人工确认";
  return {
    id: chatEntryId(gatePromptNode?.node_id ?? "human_confirm", "gate-prompt"),
    type: "gate_prompt",
    role: "system",
    content: workItemPlanContextBlockerGatePromptContent(state) ?? fallbackContent,
    timestamp: gatePromptNode?.completed_at ?? gatePromptNode?.started_at ?? new Date().toISOString(),
    node_id: gatePromptNode?.node_id,
    metadata: Object.keys(metadata).length > 0 ? metadata : undefined,
  };
}

function workItemPlanContextBlockerGatePromptContent(state: WorkspaceWsState): string | null {
  if (
    state.workspaceType !== "work_item_plan" ||
    state.workItemPlanArtifact?.type !== "context_blocker"
  ) {
    return null;
  }
  const summary = state.workItemPlanArtifact.payload.exploration_summary.trim();
  return summary || null;
}

function findLatestNodeOfType(nodes: TimelineNode[], type: TimelineNodeType) {
  for (let index = nodes.length - 1; index >= 0; index -= 1) {
    if (nodes[index].node_type === type) {
      return nodes[index];
    }
  }
  return null;
}

export function chatEntryId(nodeId: string, suffix: string) {
  return `${nodeId}:${suffix}`;
}

function textFromSources(sources: Array<string | null | undefined>) {
  for (const source of sources) {
    const trimmed = source?.trim();
    if (trimmed) {
      return trimmed;
    }
  }
  return "";
}

function executionEventContent(event: ExecutionEvent, nodeTitle?: string | null) {
  const command = event.kind === "command" ? event.command?.trim() : "";
  if (command) {
    return command;
  }
  if (isProviderPromptEvent(event) && nodeTitle) {
    return providerPromptContent(nodeTitle);
  }
  return event.detail ? `${event.title} · ${event.detail}` : event.title;
}

function providerPromptContent(nodeTitle: string) {
  return `${nodeTitle} · Provider Prompt`;
}

function formatContentSize(chars: number) {
  if (chars < 1024) {
    return `${chars} 字符`;
  }
  return `约 ${Math.ceil(chars / 1024)}KB`;
}

function isProviderPromptEvent(
  event: Pick<ExecutionEvent, "title" | "output">,
): event is Pick<ExecutionEvent, "title" | "output"> & { output: string } {
  return event.title === "Provider Prompt" && typeof event.output === "string";
}

function isProviderPromptTitle(event: Pick<ExecutionEvent, "title">) {
  return event.title === "Provider Prompt";
}

function latestProviderPromptEvent(events: ExecutionEvent[]) {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    if (isProviderPromptEvent(event)) {
      return event;
    }
  }
  return null;
}

function providerPromptEventMetadata(
  event: ExecutionEvent,
  provider?: string | null,
  node?: TimelineNode,
) {
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
    ...providerEntryMetadata(node, provider),
  };
}

export function providerEntryMetadata(
  node: TimelineNode | undefined,
  provider?: string | null,
): Record<string, unknown> | undefined {
  const metadata: Record<string, unknown> = {};
  if (provider) {
    metadata.provider = provider;
  }
  if (node?.retry) {
    metadata.retry = node.retry;
  }
  return Object.keys(metadata).length > 0 ? metadata : undefined;
}

function retriedTimelineNodeIds(nodes: TimelineNode[]) {
  return new Set(
    nodes
      .map((node) => node.retry?.retry_of_node_id)
      .filter((nodeId): nodeId is string => typeof nodeId === "string" && nodeId.length > 0),
  );
}

function providerNameForNode(node: TimelineNode, detail: TimelineNodeDetail, event?: ExecutionEvent) {
  return (
    stringField(event, "agent") ??
    node.agent ??
    stringField(detail.provider, "name")
  );
}

function isPreparedWorkspaceContextMessage(message: WsMessage) {
  return (
    message.role === "system" &&
    (message.content.startsWith("Workspace 生成任务已准备") ||
      message.content.includes("候选 spec 生成器") ||
      message.content.includes("候选 design 生成器") ||
      message.content.includes("候选 work item 生成器"))
  );
}

function stringField(value: unknown, key: string) {
  if (!isRecord(value)) {
    return null;
  }
  const field = value[key];
  return typeof field === "string" ? field : null;
}

function permissionResponseLabel(toolName: string, response: unknown) {
  if (!isRecord(response)) {
    return `权限响应 ${toolName}`;
  }

  if (response.approved === true) {
    return `已允许 ${toolName}`;
  }
  if (response.approved === false) {
    return `已拒绝 ${toolName}`;
  }
  if (response.status === "timeout") {
    return `权限超时 ${toolName}`;
  }
  return `权限响应 ${toolName}`;
}

export function choiceResponseSummary(entry: ChatEntry, response: ChoiceResponsePayload) {
  const metadata = entry.metadata;
  const labels = response.selected_option_ids.map((id) => choiceOptionLabel(metadata?.options, id));
  if (response.free_text) {
    labels.push(response.free_text);
  }
  return labels.length > 0 ? `：${labels.join("、")}` : "";
}

function choiceOptionLabel(options: unknown, id: string) {
  if (!Array.isArray(options)) {
    return id;
  }
  const option = options.find(
    (item) => isRecord(item) && stringField(item, "id") === id,
  );
  return isRecord(option) ? stringField(option, "label") ?? id : id;
}

function getStringField(value: unknown, key: string) {
  if (!isRecord(value)) {
    return null;
  }
  const field = value[key];
  return typeof field === "string" ? field : null;
}

function getArrayField(value: unknown, key: string) {
  if (!isRecord(value)) {
    return [];
  }
  const field = value[key];
  return Array.isArray(field) ? field : [];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
