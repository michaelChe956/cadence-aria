import { useState } from "react";
import type {
  ArtifactVersion,
  ExecutionEvent,
  NodeDetail,
  PermissionEvent,
  TimelineNode,
} from "../../api/types";

interface NodeDetailPanelProps {
  node: TimelineNode;
  detail: NodeDetail | null;
  artifactVersions: ArtifactVersion[];
}

type TabKey = "overview" | "streaming" | "execution" | "permission" | "artifact";

const TABS: Array<{ key: TabKey; label: string; testId: string }> = [
  { key: "overview", label: "概览", testId: "tab-overview" },
  { key: "streaming", label: "流式输出", testId: "tab-streaming" },
  { key: "execution", label: "执行事件", testId: "tab-execution" },
  { key: "permission", label: "权限", testId: "tab-permission" },
  { key: "artifact", label: "Artifact", testId: "tab-artifact" },
];

export function NodeDetailPanel({ node, detail, artifactVersions }: NodeDetailPanelProps) {
  const [activeTab, setActiveTab] = useState<TabKey>("overview");
  const artifact = findArtifact(detail, artifactVersions);

  return (
    <section
      data-testid="node-detail-panel"
      className="flex min-h-0 flex-col rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)]"
    >
      <div className="grid grid-cols-5 border-b border-[var(--aria-line)]">
        {TABS.map((tab) => (
          <button
            key={tab.key}
            type="button"
            data-testid={tab.testId}
            aria-pressed={activeTab === tab.key}
            onClick={() => setActiveTab(tab.key)}
            className={
              activeTab === tab.key
                ? "h-9 border-b-2 border-[var(--aria-primary)] px-2 text-xs font-semibold text-[var(--aria-primary)]"
                : "h-9 px-2 text-xs font-semibold text-[var(--aria-ink-muted)] hover:text-[var(--aria-ink)]"
            }
          >
            {tab.label}
          </button>
        ))}
      </div>

      <div className="min-h-0 flex-1 overflow-auto p-3">
        {activeTab === "overview" ? <OverviewTab node={node} detail={detail} /> : null}
        {activeTab === "streaming" ? <StreamingTab detail={detail} /> : null}
        {activeTab === "execution" ? <ExecutionTab events={detail?.execution_events ?? []} /> : null}
        {activeTab === "permission" ? (
          <PermissionTab events={detail?.permission_events ?? []} />
        ) : null}
        {activeTab === "artifact" ? <ArtifactTab artifact={artifact} /> : null}
      </div>
    </section>
  );
}

function OverviewTab({ node, detail }: { node: TimelineNode; detail: NodeDetail | null }) {
  return (
    <dl className="space-y-2 text-sm">
      <InfoRow label="节点" value={node.title} />
      <InfoRow label="类型" value={node.node_type} />
      <InfoRow label="状态" value={node.status} />
      <InfoRow label="阶段" value={node.stage} />
      {detail?.provider ? (
        <InfoRow label="Provider" value={`${detail.provider.name} / ${detail.provider.model}`} />
      ) : null}
      {detail?.verdict ? <InfoRow label="审核结论" value={detail.verdict.summary} /> : null}
      {detail?.artifact_ref ? <InfoRow label="Artifact" value={`v${detail.artifact_ref.version}`} /> : null}
      {detail?.is_revision ? <InfoRow label="修订" value="是" /> : null}
    </dl>
  );
}

function StreamingTab({ detail }: { detail: NodeDetail | null }) {
  return (
    <pre
      data-testid="streaming-content"
      className="min-h-32 whitespace-pre-wrap rounded-md border border-[var(--aria-line)] bg-white p-3 font-mono text-xs text-[var(--aria-ink)]"
    >
      {detail?.streaming_content || "无流式输出"}
    </pre>
  );
}

function ExecutionTab({ events }: { events: ExecutionEvent[] }) {
  if (events.length === 0) {
    return <EmptyState>无执行事件</EmptyState>;
  }

  return (
    <div className="space-y-2">
      {events.map((event) => (
        <div
          key={event.event_id}
          className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2 text-xs"
        >
          <div className="font-semibold text-[var(--aria-ink)]">{event.title}</div>
          <div className="mt-1 font-mono text-[var(--aria-ink-muted)]">
            {event.kind} / {event.status}
          </div>
          {event.command ? <div className="mt-1 font-mono">{event.command}</div> : null}
          {event.output ? <pre className="mt-2 whitespace-pre-wrap">{event.output}</pre> : null}
        </div>
      ))}
    </div>
  );
}

function PermissionTab({ events }: { events: PermissionEvent[] }) {
  if (events.length === 0) {
    return <EmptyState>无权限事件</EmptyState>;
  }

  return (
    <div className="space-y-2">
      {events.map((event) => (
        <div
          key={event.request_id}
          className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2 text-xs"
        >
          <div className="font-semibold text-[var(--aria-ink)]">{event.request_id}</div>
          <div className="mt-1 text-[var(--aria-ink-muted)]">{permissionStatus(event)}</div>
          <pre className="mt-2 max-h-28 overflow-auto whitespace-pre-wrap rounded bg-white p-2 font-mono">
            {JSON.stringify(event.request, null, 2)}
          </pre>
        </div>
      ))}
    </div>
  );
}

function ArtifactTab({ artifact }: { artifact: ArtifactVersion | null }) {
  if (!artifact) {
    return <EmptyState>无 Artifact</EmptyState>;
  }

  return (
    <pre className="min-h-32 whitespace-pre-wrap rounded-md border border-[var(--aria-line)] bg-white p-3 font-mono text-xs text-[var(--aria-ink)]">
      {artifact.markdown}
    </pre>
  );
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex min-w-0 items-center justify-between gap-3">
      <dt className="shrink-0 text-[var(--aria-ink-muted)]">{label}</dt>
      <dd className="min-w-0 truncate font-medium text-[var(--aria-ink)]">{value}</dd>
    </div>
  );
}

function EmptyState({ children }: { children: string }) {
  return <div className="text-sm text-[var(--aria-ink-muted)]">{children}</div>;
}

function findArtifact(detail: NodeDetail | null, versions: ArtifactVersion[]) {
  if (!detail?.artifact_ref) {
    return null;
  }

  return (
    versions.find(
      (version) =>
        version.source_node_id === detail.artifact_ref?.artifact_id ||
        version.version === detail.artifact_ref?.version,
    ) ?? null
  );
}

function permissionStatus(event: PermissionEvent) {
  const response = event.response;
  if (!isRecord(response)) {
    return "待应答";
  }

  if (response.approved === true) {
    return "已批准";
  }
  if (response.approved === false) {
    return "已拒绝";
  }
  if (response.status === "timeout") {
    return "超时";
  }
  return "已应答";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
