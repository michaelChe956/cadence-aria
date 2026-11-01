import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";

/// choice 语义应答策略(v2.0 Q4.4/Ruling 11):
/// 由「站内消息通知中心」案例契约固定答案,不移植矩阵单仓偏置、不盲选首项;
/// 归一仅做 Unicode/空白/编号前缀,不删否定词;未知题 fail-closed:
/// 完整题目/选项落证据后抛错,由 controller 分析,绝不随意猜。

export const FOUR_LAYERS = ["busi", "api", "gateway", "frontend"] as const;
export type LayerName = (typeof FOUR_LAYERS)[number];
/** 依赖提供者先做(v2.0 §4.2 固定依赖顺序,不称「下游先做」)。 */
export const DEPENDENCY_ORDER: readonly LayerName[] = ["busi", "api", "gateway", "frontend"];

const LAYER_ALIASES: Record<LayerName, readonly string[]> = {
  busi: ["busi", "业务层", "业务", "business", "领域层", "领域"],
  api: ["api", "接口层", "服务层", "application", "api层"],
  gateway: ["gateway", "网关", "网关层", "gateway层"],
  frontend: ["frontend", "前端", "前端层", "web", "ui"],
};

/** 归一:NFC+空白折叠+剥编号前缀(1./①/A/、/-)与全角变体。 */
export function normalizeText(value: string): string {
  return value
    .normalize("NFC")
    .replace(/\s+/g, " ")
    .replace(/^[\s]*([0-9]{1,3}[.、)]|[①②③④⑤⑥⑦⑧⑨⑩]|[A-Za-z][.、)])[ ]?/, "")
    .trim()
    .toLowerCase();
}

function matchLayer(normalized: string): LayerName | null {
  for (const layer of FOUR_LAYERS) {
    for (const alias of LAYER_ALIASES[layer]!) {
      // 词边界语义:整词或以非字母数字分隔出现,避免 api 误配 gateway 描述里的字样。
      if (new RegExp(`(^|[^a-z0-9])${alias.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}([^a-z0-9]|$)`).test(normalized)) {
        return layer;
      }
    }
  }
  return null;
}

export function matchLayerInText(text: string): LayerName | null {
  return matchLayer(normalizeText(text));
}

/** prompt 语义分类(保守:关键词组合,不搞模糊评分)。
 * 「哪个层/哪些层」是泛指层措辞,顺序题也会用;显式范围词(涉及/范围/
 * involved/四仓/多选)才判 scope;两义并存按 unknown fail-closed。 */
export function classifyPrompt(prompt: string): "four-repo-scope" | "dependency-order" | "unknown" {
  const text = normalizeText(prompt);
  const explicitScope = /(涉及|范围|仓库|成员|repos?|modules?|scope|involved|哪些层|四仓|分层|多选)/.test(text);
  const asksOrder = /(顺序|先后|优先|先做|首先|order|first|priority|依赖)/.test(text);
  if (explicitScope && asksOrder) return "unknown";
  if (asksOrder) return "dependency-order";
  if (explicitScope) return "four-repo-scope";
  return "unknown";
}

export type FailClosedEvidence = {
  stage: string;
  cause: string;
  entryText?: string;
  options?: unknown;
};

export function writeFailClosedEvidence(evidenceDir: string, evidence: FailClosedEvidence): string {
  mkdirSync(evidenceDir, { recursive: true });
  const file = path.join(evidenceDir, `choice-fail-closed-${evidence.stage}-${Date.now()}.json`);
  writeFileSync(file, JSON.stringify(evidence, null, 2), "utf8");
  return file;
}

export type LayerDecision = { layers: LayerName[]; reason: string };

/**
 * 由条目全文(prompt+题干+选项 label 合并文本)给出层选择契约答案:
 * - dependency-order(单选):busi(依赖提供者先做);
 * - four-repo-scope(多选):四层全选;
 * - 未知语义/多选形态与契约不符 → fail-closed(证据落盘后抛错)。
 */
export function decideChoiceLayers(
  entryText: string,
  options: { allowMultiple: boolean; stage: string; evidenceDir: string },
): LayerDecision {
  const kind = classifyPrompt(entryText);
  const fail = (cause: string): never => {
    const file = writeFailClosedEvidence(options.evidenceDir, {
      stage: options.stage,
      cause,
      entryText,
    });
    throw new Error(`choice 语义无法由案例契约唯一判定(${cause}):证据 ${file};fail-closed 请 controller 判定,不盲选`);
  };
  if (kind === "unknown") fail(`条目语义未命中契约分类:${entryText.slice(0, 200)}`);
  if (kind === "dependency-order") {
    if (options.allowMultiple) fail("顺序题却允许多选");
    return { layers: ["busi"], reason: "依赖提供者先做:busi→api→gateway→frontend(案例契约固定顺序)" };
  }
  if (!options.allowMultiple) fail("四仓范围题不允许多选(单选四仓范围无契约答案)");
  return { layers: [...FOUR_LAYERS], reason: "四层全仓范围:busi/api/gateway/frontend 全选(案例契约固定范围)" };
}
