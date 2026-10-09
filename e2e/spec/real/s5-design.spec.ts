import { expect, test } from "@playwright/test";
import { recordObserved } from "../../lib/run-contract.ts";
import { SessionWorkspacePage } from "../../lib/page-objects/session-workspace.page.ts";
import {
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

/// real 全旅程 S5:Design Spec 生成(codex)→ involved 四仓 choice 语义应答 →
/// 会话门确认 → durable confirmed。范围界内断言:范围题以「四层全选」应答
/// (事件台账留证);四仓收敛最终由 S6 的 work_item_repository_groups 断言。

const STAGE_TEST_TIMEOUT = 5_460_000;
const STAGE_BUDGET_MS = 5_400_000;

const bootstrap = bootstrapRealJourney();

test.describe("real 全旅程 S5 Design", () => {
  test.setTimeout(STAGE_TEST_TIMEOUT);

  test("S5 design 生成与确认", async ({ page }) => {
    requirePreviousCleared(bootstrap, "s5");
    await runStage(bootstrap, "s5", async () => {
      const workbench = await enterWorkbench(bootstrap, page);

      // story 卡(confirmed)→ drawer → 生成 Design Spec。
      const storyCards = page.getByTestId("lifecycle-card-story_spec");
      await expect(storyCards).toHaveCount(1, { timeout: 30_000 });
      await storyCards.nth(0).click();
      const drawer = page.getByTestId("lifecycle-card-drawer");
      await expect(drawer).toBeVisible({ timeout: 15_000 });
      await expect(drawer.getByTestId("drawer-status-chip")).toContainText("已确认", { timeout: 15_000 });
      const nextButton = drawer.getByTestId("drawer-generate-next");
      await expect(nextButton).toBeVisible();
      await expect(nextButton).toHaveText(/生成 Design Spec/);
      await expect(nextButton).toBeEnabled();
      await nextButton.click();

      await expect(page).toHaveURL(/\/workbench\/workspace\/[^/]+$/, { timeout: 120_000 });
      const sessionId = page.url().split("/").pop()!;
      recordObserved(bootstrap.runId, { designSessionId: sessionId });

      const session = new SessionWorkspacePage(page, sessionId);
      await expect(session.shell).toBeVisible({ timeout: 60_000 });
      // 纯 codex 形态:S4 已存默认,此处双保险显式核对/切换 author+reviewer=codex。
      await session.ensureCodexProviders(`${bootstrap.evidenceRoot}/s5-session`, "s5");
      const events = await session.driveUntilQuiet({
        stage: "s5",
        budgetMs: STAGE_BUDGET_MS,
        evidenceDir: `${bootstrap.evidenceRoot}/s5-session`,
        onHeartbeat: throttledConsole("s5"),
      });
      const scopeAnswers = events.choicesAnswered.filter((choice) => choice.reason.includes("四层全仓"));
      test.info().annotations.push({
        type: "s5-events",
        description: `gates=${events.gatesConfirmed.length} choices=${events.choicesAnswered.length} scopeAnswers=${scopeAnswers.length}(${scopeAnswers.map((answer) => answer.selected.join("/")).join(";")})`,
      });

      // durable 双面:design spec confirmed(有界轮询允许后置落盘)。
      await expect
        .poll(
          async () => {
            const lifecycle = await readIssueLifecycle(bootstrap);
            return { count: lifecycle.design_specs.length, status: lifecycle.design_specs[0]?.confirmation_status ?? "none" };
          },
          { timeout: 600_000, intervals: [5_000] },
        )
        .toEqual({ count: 1, status: "confirmed" });
      const lifecycle = await readIssueLifecycle(bootstrap);
      const design = lifecycle.design_specs[0]!;
      recordObserved(bootstrap.runId, { designSpecId: design.design_spec_id });

      await screenshotOf(bootstrap, page, "s5-design-confirmed");
      await dumpStage(bootstrap, "s5");
      expectNoDrift(bootstrap, boundarySnapshot(bootstrap, "after-s5"), "s5");
      await workbench.expectLoaded().catch(() => {});
    });
  });
});
