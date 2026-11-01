import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const [campaignInputRoot, outputFixture] = process.argv.slice(2);

if (!campaignInputRoot || !outputFixture || process.argv.length !== 4) {
  throw new Error(
    "用法：node extract_golden_findings.mjs <campaign-input-root> <output-fixture>",
  );
}

const manualAnnotationNote =
  "category、class_hint、contract_field 为人工标注，不来自 provider 原始输出。";

async function readResult(campaignRoot, run) {
  const resultPath = path.join(campaignRoot, run, "result.json");
  const content = await readFile(resultPath, "utf8");
  const result = JSON.parse(content);

  if (!Array.isArray(result.verdicts)) {
    throw new Error(`${resultPath} 缺少 verdicts 数组。`);
  }

  return result;
}

function findingsFromVerdicts(result, run) {
  return result.verdicts.flatMap((reviewVerdict) => {
    if (!Array.isArray(reviewVerdict.findings)) {
      throw new Error(`${run} 的 verdict 缺少 findings 数组。`);
    }

    return reviewVerdict.findings.map((finding) => ({
      verdict: reviewVerdict.verdict,
      finding,
    }));
  });
}

function rawFixture({ id, sourceRun, verdict, finding, expectedClass }) {
  return {
    id,
    source_run: sourceRun,
    source_kind: "provider_raw",
    verdict,
    finding,
    expected_class: expectedClass,
    expected_category: null,
    contract_field: null,
  };
}

function annotatedVariant({
  id,
  sourceRun,
  verdict,
  finding,
  category,
  contractField,
}) {
  return {
    id,
    source_run: sourceRun,
    source_kind: "annotated_variant",
    verdict,
    finding: {
      ...finding,
      category,
      class_hint: "repairable",
      contract_field: contractField,
    },
    expected_class: "repairable",
    expected_category: category,
    contract_field: contractField,
    annotation: {
      provenance: "human_annotation",
      annotated_fields: ["category", "class_hint", "contract_field"],
      note: manualAnnotationNote,
    },
  };
}

function assertCount(run, findings, expectedCount) {
  if (findings.length !== expectedCount) {
    throw new Error(
      `${run} 原始 finding 数量错误：期望 ${expectedCount}，实际 ${findings.length}。`,
    );
  }
}

function assertNoAutoRepairAllowed(value, location = "fixture") {
  if (Array.isArray(value)) {
    value.forEach((item, index) => {
      assertNoAutoRepairAllowed(item, `${location}[${index}]`);
    });
    return;
  }

  if (value && typeof value === "object") {
    for (const [key, nested] of Object.entries(value)) {
      if (key === "auto_repair_allowed") {
        throw new Error(`${location} 不得包含 auto_repair_allowed。`);
      }
      assertNoAutoRepairAllowed(nested, `${location}.${key}`);
    }
  }
}

const [rep1, rep2, rep3, rep4] = await Promise.all(
  ["rep1", "rep2", "rep3", "rep4"].map((run) =>
    readResult(campaignInputRoot, run),
  ),
);

const rep1FirstVerdict = rep1.verdicts[0];
if (
  !rep1FirstVerdict ||
  rep1FirstVerdict.verdict !== "pass" ||
  !Array.isArray(rep1FirstVerdict.findings)
) {
  throw new Error("rep1 round-1 必须是含 findings 的 pass verdict。");
}

const rep1Suggestions = rep1FirstVerdict.findings;
assertCount("rep1 round-1 pass", rep1Suggestions, 2);
if (!rep1Suggestions.every((finding) => finding.severity === "suggestion")) {
  throw new Error("rep1 round-1 pass findings 必须全部为 suggestion。");
}

const rep2Findings = findingsFromVerdicts(rep2, "rep2");
const rep3Findings = findingsFromVerdicts(rep3, "rep3");
const rep4Findings = findingsFromVerdicts(rep4, "rep4");
assertCount("rep2", rep2Findings, 6);
assertCount("rep3", rep3Findings, 1);
assertCount("rep4", rep4Findings, 2);

if (!rep2Findings.every(({ verdict }) => verdict === "needs_human")) {
  throw new Error("rep2 的原始 findings 必须全部来自 needs_human verdict。");
}
if (rep3Findings[0]?.verdict !== "revise") {
  throw new Error("rep3 的原始 finding 必须来自 revise verdict。");
}
if (!rep4Findings.every(({ verdict }) => verdict === "needs_human")) {
  throw new Error("rep4 的原始 findings 必须全部来自 needs_human verdict。");
}

const fixtures = [
  ...rep1Suggestions.map((finding, index) =>
    rawFixture({
      id: `rep1-f${index + 1}`,
      sourceRun: "rep1",
      verdict: "pass",
      finding,
      expectedClass: "advisory",
    }),
  ),
  ...rep2Findings.map(({ verdict, finding }, index) =>
    rawFixture({
      id: `rep2-f${index + 1}`,
      sourceRun: "rep2",
      verdict,
      finding,
      expectedClass: "human_required",
    }),
  ),
  rawFixture({
    id: "rep3-f1",
    sourceRun: "rep3",
    verdict: rep3Findings[0].verdict,
    finding: rep3Findings[0].finding,
    expectedClass: "human_required",
  }),
  ...rep4Findings.map(({ verdict, finding }, index) =>
    rawFixture({
      id: `rep4-f${index + 1}`,
      sourceRun: "rep4",
      verdict,
      finding,
      expectedClass: "human_required",
    }),
  ),
  annotatedVariant({
    id: "rep2-f1-annotated",
    sourceRun: "rep2",
    verdict: rep2Findings[0].verdict,
    finding: rep2Findings[0].finding,
    category: "contract_gap",
    contractField: "backend.input_contracts",
  }),
  annotatedVariant({
    id: "rep3-f1-annotated",
    sourceRun: "rep3",
    verdict: rep3Findings[0].verdict,
    finding: rep3Findings[0].finding,
    category: "self_contradiction",
    contractField: "api_404_response_semantics",
  }),
  annotatedVariant({
    id: "rep4-f1-annotated",
    sourceRun: "rep4",
    verdict: rep4Findings[0].verdict,
    finding: rep4Findings[0].finding,
    category: "self_contradiction",
    contractField: "draft_003.non_goals",
  }),
];

const rawFixtures = fixtures.filter(
  ({ source_kind: sourceKind }) => sourceKind === "provider_raw",
);
const annotatedFixtures = fixtures.filter(
  ({ source_kind: sourceKind }) => sourceKind === "annotated_variant",
);

if (rawFixtures.length !== 11 || annotatedFixtures.length !== 3 || fixtures.length !== 14) {
  throw new Error(
    `fixture 数量错误：期望 14（11 原始 + 3 标注变体），实际 ${fixtures.length}（${rawFixtures.length} 原始 + ${annotatedFixtures.length} 标注变体）。`,
  );
}

const uniqueIds = new Set(fixtures.map(({ id }) => id));
if (uniqueIds.size !== fixtures.length) {
  throw new Error("fixture id 必须唯一。");
}

assertNoAutoRepairAllowed(fixtures);

await mkdir(path.dirname(outputFixture), { recursive: true });
await writeFile(outputFixture, `${JSON.stringify(fixtures, null, 2)}\n`, "utf8");
