import { useEffect, useMemo, useRef, useState } from "react";
import type { ArtifactVersionSummary, ReviewVerdictType } from "../../../api/types";
import { MonacoViewer } from "../../shared/MonacoViewer";
import {
  REVISION_DIFF_MAX_LINES,
  computeRevisionDiff,
  toUnifiedHunks,
} from "../../../state/revision-diff";

export interface RevisionDiffViewProps {
  sessionId: string | null;
  versions: readonly ArtifactVersionSummary[];
  contentCache: Readonly<Record<number, string>>; // 版本号 -> markdown（含零个条目）
  loadVersionMarkdown: ((version: number) => Promise<string>) | null;
  onCacheVersionMarkdown: ((version: number, markdown: string) => void) | null;
  /** F-38：最近一次 review 结论的可选建议条数（>0 才渲染标签）；null 即无结论。 */
  advisoryFindingCount?: number | null;
}

// 「为什么改」：与 ReviewVerdictEntry 的 verdictLabel 同一套中文文案。
const REVIEW_VERDICT_LABELS: Readonly<Record<ReviewVerdictType, string>> = {
  pass: "通过",
  revise: "建议返修",
  needs_human: "需要人工确认",
};

export function RevisionDiffView({
  sessionId,
  versions,
  contentCache,
  loadVersionMarkdown,
  onCacheVersionMarkdown,
  advisoryFindingCount = null,
}: RevisionDiffViewProps) {
  const [baseVersion, setBaseVersion] = useState<number | null>(null);
  const [targetVersion, setTargetVersion] = useState<number | null>(null);
  const [localCache, setLocalCache] = useState<Record<number, string>>({});
  const [loadError, setLoadError] = useState<string | null>(null);

  const inFlightVersionsRef = useRef(new Set<number>());
  const loadedVersionsRef = useRef(new Set<number>());

  const sortedVersions = useMemo(
    () => [...versions].sort((left, right) => right.version - left.version),
    [versions],
  );
  // 默认取最近两轮（目标=最新、基准=次新）；两侧 select 互斥排除对方当前值
  // （baseOptions / targetOptions），版本号唯一 ⇒ 基准与目标结构性不可能相同。
  const effectiveTarget = targetVersion ?? sortedVersions[0]?.version ?? null;
  const effectiveBase = baseVersion ?? sortedVersions[1]?.version ?? null;
  const onlyVersion = sortedVersions.length === 1 ? sortedVersions[0] : null;

  const markdownFor = (versionNo: number): string | undefined => {
    // F-38：版本摘要自带 markdown（live artifact_update 落盘/单版本会话）时直接
    // 用全文，与 ArtifactPane 同一优先级；空白摘要不算内容，仍走缓存/fetch。
    const summaryMarkdown = versions.find(
      (item) => item.version === versionNo,
    )?.markdown;
    if (typeof summaryMarkdown === "string" && summaryMarkdown.trim().length > 0) {
      return summaryMarkdown;
    }
    return contentCache[versionNo] ?? localCache[versionNo];
  };

  // 需要内容的是「当前展示的轮次」：单版本=该版本全文（F-38 全文视图），
  // 多版本=对比的两轮。零版本无内容需求。
  const missingKey = (
    onlyVersion ? [onlyVersion.version] : [effectiveBase, effectiveTarget]
  )
    .filter(
      (versionNo): versionNo is number =>
        versionNo !== null && markdownFor(versionNo) === undefined,
    )
    .join(",");

  useEffect(() => {
    setLoadError(null);
    if (!loadVersionMarkdown || !missingKey) return;
    for (const versionNo of missingKey.split(",").map(Number)) {
      if (
        inFlightVersionsRef.current.has(versionNo) ||
        loadedVersionsRef.current.has(versionNo)
      ) continue;
      inFlightVersionsRef.current.add(versionNo);
      void loadVersionMarkdown(versionNo)
        .then((markdown) => {
          loadedVersionsRef.current.add(versionNo);
          setLocalCache((previous) => ({ ...previous, [versionNo]: markdown }));
          onCacheVersionMarkdown?.(versionNo, markdown);
        })
        .catch(() => setLoadError(`轮次 v${versionNo} 加载失败`))
        .finally(() => inFlightVersionsRef.current.delete(versionNo));
    }
  }, [missingKey, loadVersionMarkdown, onCacheVersionMarkdown]);

  const diff = useMemo(() => {
    if (effectiveBase === null || effectiveTarget === null) {
      return null;
    }
    const oldMarkdown = markdownFor(effectiveBase);
    const newMarkdown = markdownFor(effectiveTarget);
    if (oldMarkdown === undefined || newMarkdown === undefined) {
      return null;
    }
    return computeRevisionDiff(oldMarkdown, newMarkdown);
    // F-38：markdownFor 现在还认版本摘要自带的 markdown（live artifact_update），
    // 摘要变化必须重算 diff——versions 进依赖。
  }, [contentCache, localCache, effectiveBase, effectiveTarget, versions]);

  // 摘要直接消费 T1 导出的 summary / truncated，不从 lines 重算。
  const hunks = useMemo(
    () => (diff ? toUnifiedHunks(diff.lines) : []),
    [diff],
  );

  const baseOptions = sortedVersions.filter(
    (item) => item.version !== effectiveTarget,
  );
  const targetOptions = sortedVersions.filter(
    (item) => item.version !== effectiveBase,
  );

  const verdictChips = [effectiveBase, effectiveTarget]
    .filter(
      (versionNo): versionNo is number =>
        versionNo !== null &&
        versions.some(
          (item) => item.version === versionNo && item.review_verdict != null,
        ),
    )
    .map((versionNo) => ({
      version: versionNo,
      verdict: versions.find((item) => item.version === versionNo)!
        .review_verdict as ReviewVerdictType,
    }));

  return (
    <div data-testid="revision-diff-view" className="flex min-h-0 flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <label className="flex items-center gap-1 text-xs text-[var(--aria-ink-muted)]">
          基准轮次
          <select
            data-testid="revision-diff-base-select"
            value={effectiveBase ?? ""}
            onChange={(event) =>
              setBaseVersion(event.target.value === "" ? null : Number(event.target.value))
            }
            disabled={baseOptions.length === 0}
            className="aria-mono aria-num min-h-11 rounded-md border border-[var(--aria-line-strong)] bg-white px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            {baseOptions.map((item) => (
              <option key={item.version} value={item.version}>
                v{item.version}
              </option>
            ))}
          </select>
        </label>
        <span className="text-xs text-[var(--aria-ink-muted)]">对比</span>
        <label className="flex items-center gap-1 text-xs text-[var(--aria-ink-muted)]">
          目标轮次
          <select
            data-testid="revision-diff-target-select"
            value={effectiveTarget ?? ""}
            onChange={(event) =>
              setTargetVersion(event.target.value === "" ? null : Number(event.target.value))
            }
            disabled={targetOptions.length === 0}
            className="aria-mono aria-num min-h-11 rounded-md border border-[var(--aria-line-strong)] bg-white px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            {targetOptions.map((item) => (
              <option key={item.version} value={item.version}>
                v{item.version}
                {item.is_current ? "（当前）" : ""}
              </option>
            ))}
          </select>
        </label>
        {/* F-38 fix1（controller 实测反例）：verdictChips 由 effectiveBase/Target 派生，
            单版本模式没有对比对象，徽章不归表头——归属下方单版本区块（从版本记录的
            review_verdict 直接渲染），故单版本模式表头不渲染对比徽章。 */}
        {onlyVersion
          ? null
          : verdictChips.map((chip) => (
              <span
                key={chip.version}
                data-testid="revision-diff-verdict"
                className="aria-chip aria-mono aria-num border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]"
              >
                v{chip.version} 审批：{REVIEW_VERDICT_LABELS[chip.verdict]}
              </span>
            ))}
        {sessionId ? (
          <span className="aria-mono text-[11px] text-[var(--aria-ink-muted)]">
            {sessionId}
          </span>
        ) : null}
      </div>

      {onlyVersion ? (
        // F-38：单轮次会话没有对比对象，但产物全文与评审结论正是确认者要看的东西
        //（plan 会话首轮即停在门上）。此前只给「暂无对比对象」空话术。
        <section
          data-testid="revision-diff-single-version"
          className="flex min-h-0 flex-col gap-2"
        >
          <div className="flex flex-wrap items-center gap-2 text-xs">
            <span className="aria-chip aria-mono aria-num border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]">
              当前仅 v{onlyVersion.version}，无对比轮次
            </span>
            {/* F-38 fix1：评审结论是持久结论，从版本记录直接渲染——不依赖对比表头的
                effectiveBase/Target 派生（单版本模式两者不构成对比对）。无结论不画。 */}
            {onlyVersion.review_verdict != null ? (
              <span
                data-testid="revision-diff-verdict"
                className="aria-chip aria-mono aria-num border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]"
              >
                v{onlyVersion.version} 审批：
                {REVIEW_VERDICT_LABELS[onlyVersion.review_verdict]}
              </span>
            ) : null}
            {advisoryFindingCount !== null && advisoryFindingCount > 0 ? (
              <span
                data-testid="revision-diff-advisory-count"
                className="aria-chip aria-mono aria-num border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]"
              >
                可选建议 {advisoryFindingCount} 条
              </span>
            ) : null}
          </div>
          {loadError ? (
            <p role="alert" className="text-sm text-[var(--aria-danger)]">
              {loadError}
            </p>
          ) : markdownFor(onlyVersion.version) === undefined ? (
            <p className="text-sm text-[var(--aria-ink-muted)]">正在加载轮次内容…</p>
          ) : (
            <div className="h-[420px] overflow-hidden rounded-md border border-[var(--aria-line)]">
              <MonacoViewer
                value={markdownFor(onlyVersion.version) ?? ""}
                language="markdown"
                height="100%"
              />
            </div>
          )}
        </section>
      ) : sortedVersions.length === 0 ? (
        <p className="text-sm text-[var(--aria-ink-muted)]">
          当前会话还没有 artifact 轮次。
        </p>
      ) : loadError ? (
        <p role="alert" className="text-sm text-[var(--aria-danger)]">
          {loadError}
        </p>
      ) : !diff ? (
        <p className="text-sm text-[var(--aria-ink-muted)]">正在加载轮次内容…</p>
      ) : (
        <>
          <div
            data-testid="revision-diff-summary"
            className="flex flex-wrap items-center gap-2 text-xs"
          >
            <span className="aria-chip aria-mono aria-num border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]">
              基准 v{effectiveBase} → 目标 v{effectiveTarget}
            </span>
            <span className="aria-chip aria-mono aria-num border-[var(--aria-topo-node-done-border)] bg-[var(--aria-topo-node-done-bg)] text-[var(--aria-topo-node-done-fg)]">
              +{diff.summary.added} 新增
            </span>
            <span className="aria-chip aria-mono aria-num border-[var(--aria-topo-node-failed-border)] bg-[var(--aria-topo-node-failed-bg)] text-[var(--aria-topo-node-failed-fg)]">
              -{diff.summary.removed} 删除
            </span>
            <span className="aria-chip aria-mono aria-num border-[var(--aria-gate-open-border)] bg-[var(--aria-gate-open-bg)] text-[var(--aria-gate-open-fg)]">
              ~{diff.summary.changed} 修改
            </span>
            {diff.truncated ? (
              <span className="aria-chip border-[var(--aria-topo-node-failed-border)] bg-[var(--aria-topo-node-failed-bg)] text-[var(--aria-topo-node-failed-fg)]">
                超过 {REVISION_DIFF_MAX_LINES} 行，按整档替换呈现
              </span>
            ) : null}
          </div>
          {hunks.length === 0 ? (
            <p className="text-sm text-[var(--aria-ink-muted)]">两轮内容一致，无差异。</p>
          ) : (
            <div className="min-h-0 space-y-3 overflow-auto">
              {hunks.map((hunk, index) => (
                <section
                  key={`${hunk.header}-${index}`}
                  data-testid="revision-diff-hunk"
                  className="overflow-hidden rounded-md border border-[var(--aria-line)]"
                >
                  <div className="aria-mono aria-num border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-1 text-xs text-[var(--aria-ink-muted)]">
                    {hunk.header}
                  </div>
                  <pre className="aria-mono overflow-x-auto text-xs leading-5">
                    {hunk.lines.map((line) => (
                      <div
                        key={`${line.kind}-${line.oldLineNo}-${line.newLineNo}-${line.text}`}
                        data-testid={
                          line.kind === "added"
                            ? "revision-diff-line-added"
                            : line.kind === "removed"
                              ? "revision-diff-line-removed"
                              : "revision-diff-line-same"
                        }
                        className={
                          line.kind === "added"
                            ? "bg-[var(--aria-topo-node-done-bg)] px-3"
                            : line.kind === "removed"
                              ? "bg-[var(--aria-topo-node-failed-bg)] px-3"
                              : "px-3"
                        }
                      >
                        <span className="aria-num mr-2 inline-block w-8 text-right text-[var(--aria-ink-muted)]">
                          {line.oldLineNo ?? ""}
                        </span>
                        <span className="aria-num mr-2 inline-block w-8 text-right text-[var(--aria-ink-muted)]">
                          {line.newLineNo ?? ""}
                        </span>
                        <span className="mr-2">
                          {line.kind === "added" ? "+" : line.kind === "removed" ? "-" : " "}
                        </span>
                        {line.text}
                      </div>
                    ))}
                  </pre>
                </section>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}
