// P2-d（REQ-CRO-09）：分诊卡「三段大白话」的单一事实源——
// AI 遇到什么问题／做了什么／没能验证什么。纯函数、措辞写死在本文件，
// 便于单测（块 2 设计 §3.4）；`triage_summary` 缺失（旧门 JSON）时
// `problem` 回落 gate `description`，`did`/`unverified` 为空。
import type { CodingTriageDid, CodingTriageSummary } from "../api/types";

/** 技术详情折叠区的逐条 finding（reason_code + message 原文）。 */
export type TriageNarrativeFinding = {
  reasonCode: string;
  message: string;
};

/** 「AI 做了什么」块的渲染数据：headline 一句人话＋文件路径列表。 */
export type TriageNarrativeDid = {
  headline: string;
  changedFiles: string[];
};

export type TriageNarrative = {
  problem: string;
  findings: TriageNarrativeFinding[];
  did: TriageNarrativeDid | null;
  unverified: string[];
};
/** 「改了 N 个文件；已提交 <短SHA>」／「改了 N 个文件；未提交」（commit 短 SHA 取前 7 位，git 惯例）。 */
function didHeadline(did: CodingTriageDid): string {
  const filePart = `改了 ${did.changed_files.length} 个文件`;
  const commitPart = did.commit_sha ? `已提交 ${did.commit_sha.slice(0, 7)}` : "未提交";
  return `${filePart}；${commitPart}`;
}

export function buildTriageNarrative(
  summary: CodingTriageSummary | null | undefined,
  description: string,
): TriageNarrative {
  if (!summary) {
    return { problem: description, findings: [], did: null, unverified: [] };
  }
  const problem = summary.problem?.trim() ? summary.problem : description;
  const findings = (summary.findings ?? [])
    .filter(
      (finding) =>
        typeof finding?.reason_code === "string" &&
        typeof finding?.message === "string",
    )
    .map((finding) => ({
      reasonCode: finding.reason_code,
      message: finding.message,
    }));
  const did = summary.did
    ? {
        headline: didHeadline(summary.did),
        changedFiles: (summary.did.changed_files ?? []).filter(
          (file) => typeof file === "string",
        ),
      }
    : null;
  const unverified = (summary.unverified ?? []).filter(
    (item) => typeof item === "string" && item.trim().length > 0,
  );
  return { problem, findings, did, unverified };
}
