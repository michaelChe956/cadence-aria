import { expect, type Locator, type Page } from "@playwright/test";
import path from "node:path";
import { mkdirSync, writeFileSync } from "node:fs";
import { delay } from "../../env/wait.ts";
import { decideChoiceLayers, writeFailClosedEvidence } from "../choice-policy.ts";
import { dumpChoiceEntry } from "./session-workspace.page.ts";

/// 编码工作区页面对象(/workbench/projects/$projectId/issues/$issueId/coding/$attemptId)。
/// 驱动纪律:
/// - 首启「开始 Coding」只在 prepare_context 可见可用时点击(REQ-MTG 人工显式首启);
/// - coding-pending-gate 的动作按语义选择:验证/修复/继续类可点;
///   绝不点「跳过验证/人工放行/质量豁免」类(质量红线);未知动作集 fail-closed;
/// - 需要原因文本的门填语义原因并保留证据;
/// - choice 与会话线同款语义应答(fail-closed 落证);
/// - 终门「确认完成」只在按钮 enabled(readiness complete 且无诊断)时单击;
/// - 长段轮询采样 5s;20min 无活动报停滞;绝不终止(不点「中止」)。

const POLL_INTERVAL_MS = 5_000;
export const CODING_STAGNATION_LIMIT_MS = 20 * 60_000;

/** 门动作语义分类(保守关键词;命中多类/零类 → fail-closed)。 */
function classifyGateAction(label: string): "verify" | "fix" | "proceed" | "forbidden" | "unknown" {
  const text = label.toLowerCase();
  if (/(跳过验证|人工放行|质量豁免|豁免|bypass|skip.*verif|放行)/.test(text)) return "forbidden";
  if (/(验证|verif|检查|check|测试)/.test(text)) return "verify";
  if (/(修复|fix|repair|修订|修正|重新)/.test(text)) return "fix";
  if (/(继续|重试|retry|continue|resume|开始|start|推进)/.test(text)) return "proceed";
  return "unknown";
}

export class CodingWorkspacePage {
  readonly page: Page;
  readonly address: { projectId: string; issueId: string; attemptId: string };
  constructor(page: Page, address: { projectId: string; issueId: string; attemptId: string }) {
    this.page = page;
    this.address = address;
  }

  get shell(): Locator {
    return this.page.getByTestId("coding-workspace-page");
  }

  get startButton(): Locator {
    return this.page.getByRole("button", { name: "开始 Coding", exact: true });
  }

  get finalConfirmButton(): Locator {
    return this.page.getByRole("button", { name: "确认完成", exact: true });
  }

  get pendingGate(): Locator {
    return this.page.getByTestId("coding-pending-gate");
  }

  get choiceEntry(): Locator {
    return this.page.getByTestId("choice-request-entry");
  }

  async open(): Promise<void> {
    const { projectId, issueId, attemptId } = this.address;
    await this.page.goto(`/workbench/projects/${projectId}/issues/${issueId}/coding/${attemptId}`);
    await expect(this.shell).toBeVisible({ timeout: 60_000 });
  }

  /** 门内全部可点动作按钮(label + enabled)。 */
  async gateActions(gate: Locator): Promise<{ label: string; button: Locator }[]> {
    const buttons = gate.getByRole("button");
    const count = await buttons.count();
    const actions: { label: string; button: Locator }[] = [];
    for (let index = 0; index < count; index += 1) {
      const button = buttons.nth(index);
      const label = ((await button.innerText()).trim());
      if (label.length === 0) continue;
      if (await button.isEnabled()) actions.push({ label, button });
    }
    return actions;
  }

  /** 语义选门动作:禁绝 forbidden;按 fix>verify>proceed 优先;unknown 并存 → fail-closed。 */
  async actOnGate(gate: Locator, evidenceDir: string, stage: string): Promise<string> {
    const title = await gate.innerText().catch(() => "");
    const actions = await this.gateActions(gate);
    mkdirSync(evidenceDir, { recursive: true });
    writeFileSync(
      path.join(evidenceDir, `coding-gate-${stage}-${Date.now()}.json`),
      JSON.stringify({ stage, attempt: this.address.attemptId, title, actions: actions.map((action) => action.label) }, null, 2),
      "utf8",
    );
    if (actions.some((action) => classifyGateAction(action.label) === "forbidden")) {
      const file = writeFailClosedEvidence(evidenceDir, {
        stage,
        cause: `coding 门出现质量豁免类动作:${actions.map((action) => action.label).join(",")}`,
      });
      throw new Error(`coding 门含质量豁免动作,质量红线拒绝点击;证据 ${file}`);
    }
    const viable = actions
      .map((action) => ({ action, kind: classifyGateAction(action.label) }))
      .filter((candidate) => candidate.kind !== "unknown");
    if (viable.length === 0) {
      const file = writeFailClosedEvidence(evidenceDir, {
        stage,
        cause: `coding 门动作集无语义命中:${actions.map((action) => action.label).join(",") || "(无可用动作)"}`,
      });
      throw new Error(`coding 门动作集未知,fail-closed;证据 ${file}`);
    }
    const kinds = new Set(viable.map((candidate) => candidate.kind));
    if (kinds.size > 1) {
      const file = writeFailClosedEvidence(evidenceDir, {
        stage,
        cause: `coding 门动作多类并存(${[...kinds].join(",")}),优先级策略未授权消歧`,
      });
      throw new Error(`coding 门动作多类并存 fail-closed;证据 ${file}`);
    }
    if (viable.length > 1) {
      const file = writeFailClosedEvidence(evidenceDir, {
        stage,
        cause: `同类动作 ${viable.length} 个:${viable.map((candidate) => candidate.action.label).join(",")}`,
      });
      throw new Error(`coding 门同类动作歧义 fail-closed;证据 ${file}`);
    }
    const chosen = viable[0]!;
    const reasonBox = gate.getByRole("textbox");
    if ((await reasonBox.count()) > 0) {
      await reasonBox.nth(0).fill("页面 E2E 语义应答:按门语义继续推进验证/修复链,不豁免质量门");
    }
    await chosen.action.button.click();
    await expect(gate).toBeHidden({ timeout: 60_000 });
    return chosen.action.label;
  }

  /** 与会话线同款:单题语义应答(层匹配)。 */
  async answerChoice(entry: Locator, evidenceDir: string, stage: string): Promise<{ reason: string; selected: string[] }> {
    const dump = await dumpChoiceEntry(entry);
    const inputNames = [...new Set(dump.options.map((option) => option.name))];
    if (inputNames.length !== 1 || dump.hasFreeText || dump.inputType === "mixed") {
      const file = writeFailClosedEvidence(evidenceDir, {
        stage,
        cause: `coding choice 结构未获单题授权:name 组=${inputNames.length} freeText=${dump.hasFreeText} type=${dump.inputType}`,
        entryText: dump.entryText,
        options: dump.options,
      });
      throw new Error(`coding choice 结构 fail-closed;证据 ${file}`);
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
          cause: `coding choice 层 ${layer} 匹配数=${matches.length}`,
          entryText: dump.entryText,
          options: dump.options,
        });
        throw new Error(`coding choice 层 ${layer} 歧义 fail-closed;证据 ${file}`);
      }
      const target = matches[0]!;
      await entry.locator(`input[name="${target.name}"]`).nth(target.index).check();
      selectedLabels.push(target.label);
    }
    await entry.getByRole("button", { name: "提交选择" }).click();
    await expect(entry.getByRole("button", { name: "提交选择" })).toBeHidden({ timeout: 30_000 });
    return { reason: decision.reason, selected: selectedLabels };
  }

  async activitySignature(): Promise<string> {
    return this.page
      .evaluate(() => {
        const root = document.querySelector('[data-testid="coding-workspace-page"]');
        const gates = document.querySelectorAll('[data-testid="coding-pending-gate"]').length;
        const choices = document.querySelectorAll('[data-testid="choice-request-entry"]').length;
        const status = document.querySelector('[data-testid="coding-status-bar"]')?.textContent?.trim() ?? "";
        return `${gates}:${choices}:${status}:${root ? root.textContent?.length ?? 0 : 0}`;
      })
      .catch(() => "evaluate-failed");
  }

  /**
   * 驱动单个 attempt 直到终门确认完成(单击 enabled 的「确认完成」)
   * 或预算耗尽。返回事件台账。等待终门期间 readiness 未满则持续轮询。
   */
  async driveToFinalConfirm(options: {
    stage: string;
    budgetMs: number;
    evidenceDir: string;
    onHeartbeat?: (waitedMs: number, signature: string) => void;
  }): Promise<{ started: boolean; gatesActed: string[]; choicesAnswered: number; finalConfirmedAt: string | null }> {
    const deadline = Date.now() + options.budgetMs;
    const startedAt = Date.now();
    let started = false;
    let lastActivityAt = Date.now();
    let lastSignature = "";
    const gatesActed: string[] = [];
    let choicesAnswered = 0;
    for (;;) {
      if (!started && (await this.startButton.count()) > 0 && (await this.startButton.nth(0).isEnabled())) {
        if ((await this.startButton.count()) !== 1) throw new Error("开始 Coding 按钮歧义(>1)");
        await this.startButton.nth(0).click();
        started = true;
        lastActivityAt = Date.now();
        continue;
      }
      const gate = this.pendingGate;
      if ((await gate.count()) > 0 && (await gate.nth(0).isVisible())) {
        if ((await gate.count()) > 1) throw new Error(`coding 门歧义:同时可见 ${await gate.count()} 个`);
        gatesActed.push(await this.actOnGate(gate.nth(0), options.evidenceDir, options.stage));
        lastActivityAt = Date.now();
        continue;
      }
      const choiceCount = await this.choiceEntry.count();
      if (choiceCount > 0) {
        const entry = this.choiceEntry.nth(0);
        const submit = entry.getByRole("button", { name: "提交选择" });
        if ((await submit.count()) > 0 && (await submit.isEnabled())) {
          await this.answerChoice(entry, options.evidenceDir, options.stage);
          choicesAnswered += 1;
          lastActivityAt = Date.now();
          continue;
        }
      }
      const finalCount = await this.finalConfirmButton.count();
      if (finalCount > 0) {
        const button = this.finalConfirmButton.nth(0);
        if (await button.isEnabled()) {
          if (finalCount !== 1) throw new Error(`确认完成按钮数=${finalCount},歧义拒绝点击`);
          await button.click();
          await expect(this.page.getByTestId("coding-final-confirm-waiting")).toBeHidden({ timeout: 60_000 }).catch(() => {});
          return { started, gatesActed, choicesAnswered, finalConfirmedAt: new Date().toISOString() };
        }
      }
      const signature = await this.activitySignature();
      if (signature !== lastSignature) {
        lastSignature = signature;
        lastActivityAt = Date.now();
      }
      options.onHeartbeat?.(Date.now() - startedAt, signature);
      if (Date.now() - lastActivityAt >= CODING_STAGNATION_LIMIT_MS) {
        throw new Error(
          `coding 停滞 ${Math.round((Date.now() - lastActivityAt) / 60_000)}min 无页面活动(attempt=${this.address.attemptId});保留现场`,
        );
      }
      if (Date.now() >= deadline) {
        throw new Error(`coding 段预算耗尽(${Math.round(options.budgetMs / 60_000)}min,attempt=${this.address.attemptId});失败关闭`);
      }
      await delay(POLL_INTERVAL_MS);
    }
  }
}
