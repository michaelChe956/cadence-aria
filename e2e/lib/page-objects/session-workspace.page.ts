import { expect, type Locator, type Page } from "@playwright/test";
import path from "node:path";
import { mkdirSync, writeFileSync } from "node:fs";
import { delay } from "../../env/wait.ts";
import { decideChoiceLayers, matchLayerInText, writeFailClosedEvidence } from "../choice-policy.ts";

type ParsedChoiceOption = { name: string; index: number; label: string; description: string; layer: string | null };
type ParsedChoiceEntry = { entryText: string; inputType: "radio" | "checkbox" | "mixed"; options: ParsedChoiceOption[]; hasFreeText: boolean };

/// 解析 choice 条目 DOM:输入按 name 分组(题),label 文本语义匹配层。
export async function dumpChoiceEntry(entry: Locator): Promise<ParsedChoiceEntry> {
  type DomDump = { entryText: string; inputType: "radio" | "checkbox" | "mixed"; hasFreeText: boolean; options: { name: string; index: number; label: string; description: string }[] };
  const parsed = await entry.evaluate<DomDump>((root) => {
    const options: { name: string; index: number; label: string; description: string }[] = [];
    const inputs = Array.from(root.querySelectorAll("label > input[type=checkbox], label > input[type=radio]"));
    inputs.forEach((input, index) => {
      const label = input.parentElement;
      const spans = label ? Array.from(label.querySelectorAll("span")) : [];
      options.push({
        name: input.getAttribute("name") ?? "",
        index,
        label: spans[0]?.textContent?.trim() ?? label?.textContent?.trim() ?? "",
        description: spans[1]?.textContent?.trim() ?? "",
      });
    });
    const typeSet = new Set(inputs.map((input) => input.getAttribute("type")));
    return {
      entryText: root.textContent?.trim() ?? "",
      inputType: typeSet.size === 1 ? (typeSet.has("checkbox") ? "checkbox" : "radio") : "mixed",
      hasFreeText: root.querySelector("textarea") !== null,
      options,
    };
  });
  const options = parsed.options.map((option) => ({
    ...option,
    layer: matchLayerInText(`${option.label} ${option.description}`),
  }));
  return { ...parsed, options };
}
/// 会话工作区页面对象(/workbench/workspace/$sessionId → cockpit)。
/// 驱动纪律(v2.0 Q4):
/// - 只操作可见、未解决、enabled 的当前门/choice;多于一个候选即失败,不 .first();
/// - 门确认=GatePromptEntry 内「确认当前版本」(单击语义;终止类才是
///   confirm-twice-button 双击,本旅程绝不点终止);
/// - choice 语义应答见 lib/choice-policy.ts(fail-closed 落证据);
/// - 长段等待=有界轮询采样(5s),以页面活动(门/choice 事件/流式内容变化)
///   重置停滞计时;20min 无活动如实报停滞(不杀进程,交上层裁决)。

const POLL_INTERVAL_MS = 5_000;
export const STAGNATION_LIMIT_MS = 20 * 60_000;

export type DriveEvents = {
  gatesConfirmed: { at: string; context: string }[];
  choicesAnswered: { at: string; reason: string; selected: string[] }[];
};

export class SessionWorkspacePage {
  readonly page: Page;
  readonly sessionId: string;

  constructor(page: Page, sessionId: string) {
    this.page = page;
    this.sessionId = sessionId;
  }

  get shell(): Locator {
    return this.page.getByTestId("cockpit-page");
  }

  get gateEntry(): Locator {
    return this.page.getByTestId("gate-prompt-entry");
  }

  get choiceEntry(): Locator {
    return this.page.getByTestId("choice-request-entry");
  }

  async open(): Promise<void> {
    await this.page.goto(`/workbench/workspace/${this.sessionId}`);
    await expect(this.shell).toBeVisible({ timeout: 60_000 });
  }

  /**
   * 会话 cockpit 角色级 provider 显式选择(纯 codex 形态):
   * Author/Reviewer 均 select codex(Reviewer 需「启用交叉审核」开);
   * 随后点 save-provider-defaults 把默认落 localStorage(后续生成请求
   * 按 REQ-PPS-01 快照 codex/codex)。无配置面(canConfigureProviders
   * false)时如实抛错,不静默跳过。
   */
  async ensureCodexProviders(evidenceDir: string, stage: string): Promise<void> {
    const openButton = this.page.getByRole("button", { name: "Provider 配置" });
    await expect(openButton).toBeVisible({ timeout: 30_000 });
    await openButton.click();
    const dialog = this.page.getByRole("dialog", { name: "Provider 配置" });
    await expect(dialog).toBeVisible({ timeout: 15_000 });
    const authorSelect = dialog.getByRole("combobox", { name: "Author" });
    await expect(authorSelect).toBeEnabled();
    await authorSelect.selectOption({ value: "codex" });
    const reviewerToggle = dialog.getByRole("checkbox", { name: /启用交叉审核/ });
    if ((await reviewerToggle.count()) > 0 && !(await reviewerToggle.isChecked())) {
      await reviewerToggle.check();
    }
    const reviewerSelect = dialog.getByRole("combobox", { name: "Reviewer" });
    await expect(reviewerSelect).toBeVisible({ timeout: 15_000 });
    await expect(reviewerSelect).toBeEnabled();
    await reviewerSelect.selectOption({ value: "codex" });
    const selected = {
      author: await authorSelect.inputValue(),
      reviewer: await reviewerSelect.inputValue(),
    };
    await dialog.getByRole("button", { name: "关闭 Provider 配置" }).click();
    await expect(dialog).toBeHidden({ timeout: 15_000 });
    const saveDefaults = this.page.getByTestId("save-provider-defaults");
    if (await saveDefaults.isVisible()) {
      await saveDefaults.click();
    }
    mkdirSync(evidenceDir, { recursive: true });
    writeFileSync(
      path.join(evidenceDir, `provider-select-${stage}.json`),
      JSON.stringify({ stage, sessionId: this.sessionId, selected, defaultsSaved: await saveDefaults.isVisible() }, null, 2),
      "utf8",
    );
    if (selected.author !== "codex" || selected.reviewer !== "codex") {
      throw new Error(`provider 选择未收敛到 codex/codex:${JSON.stringify(selected)}`);
    }
  }

  /** 未解决门(可见且含可点确认按钮);含已解决历史门则过滤掉。 */
  async unresolvedGate(): Promise<Locator | null> {
    const candidates = this.gateEntry;
    const count = await candidates.count();
    if (count === 0) return null;
    for (let index = 0; index < count; index += 1) {
      const entry = candidates.nth(index);
      if (!(await entry.isVisible())) continue;
      const button = entry.getByRole("button", { name: /确认当前版本|确认全部|批量确认/ });
      if ((await button.count()) > 0) {
        if (count > 1) {
          throw new Error(`门歧义:同时可见 ${count} 个未解决门,身份不可唯一对应,拒绝 .first() 掩盖`);
        }
        return entry;
      }
    }
    return null;
  }

  async confirmGate(entry: Locator, evidenceDir: string, stage: string): Promise<string> {
    const contextText = await entry
      .getByTestId("gate-artifact-context")
      .innerText()
      .catch(() => "");
    const whyText = await entry.getByTestId("gate-why").innerText().catch(() => "");
    const metaText = await entry.getByTestId("gate-meta").innerText().catch(() => "");
    mkdirSync(evidenceDir, { recursive: true });
    writeFileSync(
      path.join(evidenceDir, `gate-${stage}-${Date.now()}.json`),
      JSON.stringify({ stage, sessionId: this.sessionId, why: whyText, context: contextText, meta: metaText }, null, 2),
      "utf8",
    );
    const button = entry.getByRole("button", { name: /确认当前版本|确认全部|批量确认/ });
    // 歧义守卫在 unresolvedGate 已保证唯一;此处仍要求恰好一个可点确认。
    const clickable = await button.count();
    if (clickable !== 1) {
      throw new Error(`门确认按钮数=${clickable},预期恰好 1(多批量子按钮需人工核对)`);
    }
    await button.nth(0).click();
    // 确认后门应解决(隐藏或确认按钮消失);durable 由 spec 层 GET 核对。
    await expect(entry.getByRole("button", { name: /确认当前版本|确认全部|批量确认/ })).toBeHidden({
      timeout: 30_000,
    });
    return metaText || whyText || contextText;
  }

  /** 未解决 choice(可见且提交可用)。 */
  async unresolvedChoice(): Promise<Locator | null> {
    const candidates = this.choiceEntry;
    const count = await candidates.count();
    if (count === 0) return null;
    let found = 0;
    let entry: Locator | null = null;
    for (let index = 0; index < count; index += 1) {
      const candidate = candidates.nth(index);
      if (!(await candidate.isVisible())) continue;
      const submit = candidate.getByRole("button", { name: "提交选择" });
      if ((await submit.count()) > 0 && (await submit.isEnabled())) {
        found += 1;
        entry = candidate;
      }
    }
    if (found > 1) {
      throw new Error(`choice 歧义:同时可见 ${found} 个未解决 choice,拒绝猜测`);
    }
    return entry;
  }

  /**
   * 语义应答一个 choice 条目:
   * - 按输入 name 分组题;>1 组(多题)→ fail-closed 落证;
   * - 单题:选项 label 语义匹配层,按策略勾选 busi(单选)或四层(多选);
   * - label 歧义(某层 0 或 >1 匹配)→ fail-closed 落证。
   */
  async answerChoice(entry: Locator, evidenceDir: string, stage: string): Promise<{ reason: string; selected: string[] }> {
    const dump = await dumpChoiceEntry(entry);
    const evidencePayload = {
      entryText: dump.entryText,
      inputType: dump.inputType,
      hasFreeText: dump.hasFreeText,
      options: dump.options,
    };
    const inputNames = [...new Set(dump.options.map((option) => option.name))];
    if (inputNames.length > 1 || inputNames.length === 0 || dump.hasFreeText || dump.inputType === "mixed") {
      const file = writeFailClosedEvidence(evidenceDir, {
        stage,
        cause: `choice 结构未获单题授权:name 组=${inputNames.length} freeText=${dump.hasFreeText} inputType=${dump.inputType}`,
        ...evidencePayload,
      });
      throw new Error(`choice 结构 fail-closed(多题/自由文本/混合控件):证据 ${file}`);
    }
    const decision = decideChoiceLayers(dump.entryText, {
      allowMultiple: dump.inputType === "checkbox",
      stage,
      evidenceDir,
    });
    const selectedLabels: string[] = [];
    for (const layer of decision.layers) {
      const matches = dump.options.filter((option) => option.layer === layer);
      if (matches.length !== 1) {
        const file = writeFailClosedEvidence(evidenceDir, {
          stage,
          cause: `层 ${layer} 选项匹配数=${matches.length}`,
          ...evidencePayload,
        });
        throw new Error(`choice 层 ${layer} 匹配数=${matches.length},歧义 fail-closed;证据 ${file}`);
      }
      const target = matches[0]!;
      await entry.locator(`input[name="${target.name}"]`).nth(target.index).check();
      selectedLabels.push(target.label);
    }
    await entry.getByRole("button", { name: "提交选择" }).click();
    await expect(entry.getByRole("button", { name: "提交选择" })).toBeHidden({ timeout: 30_000 });
    return { reason: decision.reason, selected: selectedLabels };
  }

  /** 页面活动签名(门/choice 计数 + 流式内容长度),用于停滞判定。 */
  async activitySignature(): Promise<string> {
    return this.page
      .evaluate(() => {
        const cockpit = document.querySelector('[data-testid="cockpit-page"]');
        const gates = document.querySelectorAll('[data-testid="gate-prompt-entry"]').length;
        const choices = document.querySelectorAll('[data-testid="choice-request-entry"]').length;
        const textLength = cockpit ? cockpit.textContent?.length ?? 0 : 0;
        return `${gates}:${choices}:${textLength}`;
      })
      .catch(() => "evaluate-failed");
  }

  /**
   * 驱动长段直到安静(门/choice 全解决且页面签名在 quietWindowMs 内无变化)
   * 或预算耗尽。返回事件台账;异常(停滞/歧义/协议错误)如实抛出。
   */
  async driveUntilQuiet(options: {
    stage: string;
    budgetMs: number;
    quietWindowMs?: number;
    evidenceDir: string;
    onHeartbeat?: (waitedMs: number, signature: string) => void;
  }): Promise<DriveEvents> {
    const quietWindowMs = options.quietWindowMs ?? 180_000;
    const deadline = Date.now() + options.budgetMs;
    const events: DriveEvents = { gatesConfirmed: [], choicesAnswered: [] };
    let lastActivityAt = Date.now();
    let lastSignature = "";
    for (;;) {
      const gate = await this.unresolvedGate();
      if (gate) {
        const context = await this.confirmGate(gate, options.evidenceDir, options.stage);
        events.gatesConfirmed.push({ at: new Date().toISOString(), context });
        lastActivityAt = Date.now();
        continue;
      }
      const choice = await this.unresolvedChoice();
      if (choice) {
        const answered = await this.answerChoice(choice, options.evidenceDir, options.stage);
        events.choicesAnswered.push({ at: new Date().toISOString(), ...answered });
        lastActivityAt = Date.now();
        continue;
      }
      const signature = await this.activitySignature();
      if (signature !== lastSignature) {
        lastSignature = signature;
        lastActivityAt = Date.now();
      }
      const waitedMs = Date.now() - (deadline - options.budgetMs);
      options.onHeartbeat?.(waitedMs, signature);
      if (Date.now() - lastActivityAt >= quietWindowMs) {
        return events; // 安静窗口达成:AI 段落收口,durable 由 spec 层核对。
      }
      if (Date.now() - lastActivityAt >= STAGNATION_LIMIT_MS) {
        throw new Error(
          `会话停滞 ${Math.round((Date.now() - lastActivityAt) / 60_000)}min 无页面活动(session=${this.sessionId});保留现场,上层裁决`,
        );
      }
      if (Date.now() >= deadline) {
        throw new Error(`段预算耗尽(${Math.round(options.budgetMs / 60_000)}min,session=${this.sessionId});失败关闭`);
      }
      await delay(POLL_INTERVAL_MS);
    }
  }
}
