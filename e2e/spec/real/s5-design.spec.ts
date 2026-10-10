import { expect, test } from "@playwright/test";
import { readManifest, recordObserved } from "../../lib/run-contract.ts";
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
  type JourneyBootstrap,
} from "../../lib/journey-support.ts";
import type { Page } from "@playwright/test";

/// real 全旅程 S5:Design Spec 生成(codex)→ involved 四仓 choice 语义应答 →
/// 会话门确认 → durable confirmed。范围界内断言:范围题以「四层全选」应答
/// (事件台账留证);四仓收敛最终由 S6 的 work_item_repository_groups 断言。
///
/// UI 语义(与 S4 的 story 队列生成不同):story 卡 drawer 的「生成 Design Spec」
/// 不直接导航——产品先创建 draft design + 会话并把 drawer 聚焦到新 design 卡,
/// 用户在 drawer 点「打开 Workspace」才进入会话页。单阶段工作区默认阶段=
/// 「需要动作的最早阶段」,取 story 卡前需先切 Story 阶段面板。

const STAGE_TEST_TIMEOUT = 5_460_000;
const STAGE_BUDGET_MS = 5_400_000;

const bootstrap = bootstrapRealJourney();

/** durable 双面:design spec confirmed(有界轮询允许后置落盘)。 */
async function assertDesignConfirmed(bootstrap: JourneyBootstrap): Promise<void> {
  await expect
    .poll(
      async () => {
        const lifecycle = await readIssueLifecycle(bootstrap);
        return { count: lifecycle.design_specs.length, status: lifecycle.design_specs[0]?.confirmation_status ?? "none" };
      },
      { timeout: 600_000, intervals: [5_000] },
    )
    .toEqual({ count: 1, status: "confirmed" });
}

/** 驱动 design 会话到定稿(纯 codex+三通道),然后 durable 断言+证据收口。 */
async function driveDesignToConfirm(bootstrap: JourneyBootstrap, session: SessionWorkspacePage, page: Page): Promise<void> {
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

  await assertDesignConfirmed(bootstrap);
  const lifecycle = await readIssueLifecycle(bootstrap);
  const design = lifecycle.design_specs[0]!;
  recordObserved(bootstrap.runId, { designSpecId: design.design_spec_id });

  await screenshotOf(bootstrap, page, "s5-design-confirmed");
  await dumpStage(bootstrap, "s5");
  expectNoDrift(bootstrap, boundarySnapshot(bootstrap, "after-s5"), "s5");
}

test.describe("real 全旅程 S5 Design", () => {
  test.setTimeout(STAGE_TEST_TIMEOUT);

  test("S5 design 生成与确认", async ({ page }) => {
    requirePreviousCleared(bootstrap, "s5");
    await runStage(bootstrap, "s5", async () => {
      // 阶段化重放(S4 同款):已 confirmed → 仅复核 durable;draft design +
      // 既有 design 会话(台账或 lifecycle 投影)→ 直接重进,不重复「生成」
      // (重复生成会产生第二个 design spec,破坏 count:1 断言)。
      const manifest = readManifest(bootstrap.runId);
      const lifecycle0 = await readIssueLifecycle(bootstrap).catch(() => null);
      const existingDesign = lifecycle0?.design_specs.at(0) ?? null;
      const designSession =
        lifecycle0?.workspace_sessions.find(
          (session) => session.workspace_type === "design" && session.entity_id === existingDesign?.design_spec_id,
        ) ?? null;
      const existingSessionId = manifest.observed.designSessionId ?? designSession?.workspace_session_id ?? null;

      if (existingDesign?.confirmation_status === "confirmed") {
        test.info().annotations.push({ type: "s5-resume", description: "design 已 confirmed,仅复核 durable" });
        if (existingSessionId) recordObserved(bootstrap.runId, { designSessionId: existingSessionId });
        recordObserved(bootstrap.runId, { designSpecId: existingDesign.design_spec_id });
        await assertDesignConfirmed(bootstrap);
        return;
      }
      if (existingDesign && existingSessionId) {
        test.info().annotations.push({ type: "s5-resume", description: `重进既有 design 会话 ${existingSessionId}` });
        recordObserved(bootstrap.runId, { designSessionId: existingSessionId });
        const session = new SessionWorkspacePage(page, existingSessionId);
        await session.open();
        await driveDesignToConfirm(bootstrap, session, page);
        return;
      }

      const workbench = await enterWorkbench(bootstrap, page);

      // story 卡(confirmed)→ drawer → 生成 Design Spec。单阶段面板:进入时
      // 已有 confirmed Story、无 Design → 默认阶段=design,story 卡只在
      // Story 面板渲染,先切阶段再取卡。
      await workbench.openStage("story");
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

      // 产品语义:生成即建 draft design + 会话,drawer 聚焦新 design 卡(URL
      // focus 切到 design_spec),不直接导航;真实用户路径 = 在 drawer 点
      // 「打开 Workspace」进入会话页。
      await expect(page).toHaveURL(/focus=design_spec/, { timeout: 120_000 });
      await expect(drawer.getByTestId("drawer-status-chip")).toContainText("草稿", { timeout: 15_000 });
      const openWorkspaceButton = drawer.getByTestId("drawer-open-workspace");
      await expect(openWorkspaceButton).toBeVisible({ timeout: 15_000 });
      await openWorkspaceButton.click();
      await expect(page).toHaveURL(/\/workbench\/workspace\/[^/]+$/, { timeout: 120_000 });
      const sessionId = page.url().split("/").pop()!;
      recordObserved(bootstrap.runId, { designSessionId: sessionId });

      const session = new SessionWorkspacePage(page, sessionId);
      await expect(session.shell).toBeVisible({ timeout: 60_000 });
      await driveDesignToConfirm(bootstrap, session, page);
    });
  });
});
