import { mkdirSync, writeFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { recordObserved, type ObservedWorkItem } from "../../lib/run-contract.ts";
import { SessionWorkspacePage } from "../../lib/page-objects/session-workspace.page.ts";
import {
  FOUR_LAYERS,
  bootstrapRealJourney,
  boundarySnapshot,
  dumpStage,
  enterWorkbench,
  expectNoDrift,
  readIssueLifecycle,
  requirePreviousCleared,
  runStage,
  screenshotOf,
  throttledConsole,
} from "../../lib/journey-support.ts";

/// real 全旅程 S6:Design(confirmed)→ Work Item Plan 准备对话框(provider
/// 显式选 codex)→ plan 会话生成 → SC 门确认 → durable:≥4 业务 WI、
/// target 覆盖四仓、全部 plan_status=confirmed。
/// 已知产品前置风险:多仓 SC prepare 曾为 SINGLE_CANDIDIDATE_PREFLIGHT_FAILED
/// 硬阻断(v2.0 E11);多仓 preflight 上线后四仓 design 应放行——若撞新层,
/// 对话框/会话错误原文落证后 FAIL 如实上报,不绕行。

const STAGE_TEST_TIMEOUT = 5_460_000;
const STAGE_BUDGET_MS = 5_400_000;

const bootstrap = bootstrapRealJourney();

test.describe("real 全旅程 S6 Plan", () => {
  test.setTimeout(STAGE_TEST_TIMEOUT);

  test("S6 plan 准备/生成/确认(四仓覆盖)", async ({ page }) => {
    requirePreviousCleared(bootstrap, "s6");
    await runStage(bootstrap, "s6", async () => {
      const workbench = await enterWorkbench(bootstrap, page);

      // design 卡(confirmed)→ drawer → 生成 Work Item(打开 Plan 选项对话框)。
      // 单阶段面板:进入时已有 Story+Design、无 Work Item → 默认阶段=work_item,
      // design 卡只在 Design 面板渲染,先切阶段再取卡。
      await workbench.openStage("design");
      const designCards = page.getByTestId("lifecycle-card-design_spec");
      await expect(designCards).toHaveCount(1, { timeout: 30_000 });
      await designCards.nth(0).click();
      const drawer = page.getByTestId("lifecycle-card-drawer");
      await expect(drawer).toBeVisible({ timeout: 15_000 });
      await expect(drawer.getByTestId("drawer-status-chip")).toContainText("已确认", { timeout: 15_000 });
      const nextButton = drawer.getByTestId("drawer-generate-next");
      await expect(nextButton).toBeVisible();
      await expect(nextButton).toHaveText(/生成 Work Item/);
      await expect(nextButton).toBeEnabled();
      await nextButton.click();

      // Plan 选项对话框:显式 provider=codex(author/reviewer);选项勾选保持默认。
      const dialog = page.locator('[aria-label="Work Item Plan 配置"]');
      await expect(dialog).toBeVisible({ timeout: 15_000 });
      const authorSelect = dialog.getByRole("combobox", { name: "Author Provider" });
      const reviewerSelect = dialog.getByRole("combobox", { name: "Reviewer Provider" });
      await authorSelect.selectOption({ value: "codex" });
      await reviewerSelect.selectOption({ value: "codex" });
      await dialog.getByRole("button", { name: "创建并打开 Workspace" }).click();

      // 提交失败(如多仓 preflight 新层)留在对话框:错误原文落证后 FAIL。
      try {
        await expect(page).toHaveURL(/\/workbench\/workspace\/[^/]+$/, { timeout: 120_000 });
      } catch {
        const alerts = await dialog.getByRole("alert").allInnerTexts().catch(() => []);
        const dialogText = await dialog.innerText().catch(() => "");
        mkdirSync(`${bootstrap.evidenceRoot}/s6-preflight`, { recursive: true });
        const file = `${bootstrap.evidenceRoot}/s6-preflight/prepare-error-${Date.now()}.json`;
        writeFileSync(file, JSON.stringify({ alerts, dialogText }, null, 2), "utf8");
        throw new Error(`S6 plan 准备未打开 workspace(疑似多仓 preflight 新层):证据 ${file};alerts=${alerts.join("|")}`);
      }
      const sessionId = page.url().split("/").pop()!;
      recordObserved(bootstrap.runId, { planSessionId: sessionId });

      // plan 会话:候选面板(author_confirm 阶段)→ SC 门确认(driver 统一门语义)。
      const session = new SessionWorkspacePage(page, sessionId);
      await expect(session.shell).toBeVisible({ timeout: 60_000 });
      const events = await session.driveUntilQuiet({
        stage: "s6",
        budgetMs: STAGE_BUDGET_MS,
        evidenceDir: `${bootstrap.evidenceRoot}/s6-session`,
        onHeartbeat: throttledConsole("s6"),
      });
      test.info().annotations.push({
        type: "s6-events",
        description: `gates=${events.gatesConfirmed.length} choices=${events.choicesAnswered.length}`,
      });

      // durable 双面:≥4 业务 WI + 四仓覆盖 + 全 confirmed(有界轮询允许后置落盘)。
      await expect
        .poll(
          async () => {
            const polled = await readIssueLifecycle(bootstrap);
            const confirmed = polled.work_items.filter((item) => item.plan_status === "confirmed").length;
            return polled.work_items.length >= 4 && confirmed === polled.work_items.length;
          },
          { timeout: 600_000, intervals: [5_000] },
        )
        .toBe(true);
      const lifecycle = await readIssueLifecycle(bootstrap);
      expect(
        lifecycle.work_items.length,
        `业务 WI 应 ≥4,实际 ${lifecycle.work_items.length}`,
      ).toBeGreaterThanOrEqual(4);
      const groups = lifecycle.work_item_repository_groups.filter((group) => group.target_repository_id !== null);
      const aliases = new Set(groups.map((group) => group.alias));
      for (const layer of FOUR_LAYERS) {
        expect(aliases, `target 分组缺 ${layer}(现有:${[...aliases].join(",")})`).toContain(layer);
      }
      for (const item of lifecycle.work_items) {
        expect(
          item.plan_status,
          `WI ${item.work_item_id} plan_status=${item.plan_status}(应 confirmed)`,
        ).toBe("confirmed");
      }
      const observedItems: ObservedWorkItem[] = lifecycle.work_items.map((item) => {
        const alias = groups.find((group) => group.target_repository_id === item.repository_id)?.alias ?? null;
        return {
          workItemId: item.work_item_id,
          alias,
          repositoryId: item.repository_id,
          attemptId: item.latest_attempt?.attempt_id ?? null,
          executionStatus: item.execution_status,
          completionCommit: item.completion_commit,
        };
      });
      recordObserved(bootstrap.runId, { workItems: observedItems });

      await screenshotOf(bootstrap, page, "s6-plan-confirmed");
      await dumpStage(bootstrap, "s6");
      expectNoDrift(bootstrap, boundarySnapshot(bootstrap, "after-s6"), "s6");
      await workbench.expectLoaded().catch(() => {});
    });
  });
});
