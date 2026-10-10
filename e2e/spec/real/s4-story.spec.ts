import { expect, test, type Page } from "@playwright/test";
import { readManifest, recordObserved } from "../../lib/run-contract.ts";
import { SessionWorkspacePage } from "../../lib/page-objects/session-workspace.page.ts";
import type { WorkbenchPage } from "../../lib/page-objects/workbench.page.ts";
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

/// real 全旅程 S4:Story Spec 生成(codex 真会话)→ 会话门确认 → durable confirmed。
/// provider 面向:无 localStorage 注入,author 走服务端默认 codex
/// (provider_workspace_config 默认 author=codex/reviewer=claude_code),
/// 实际 provider 快照经 backend dump 留证。

/// Story/Design 长段预算:5400s 失败关闭上界(v2.0 §5.2),非时长预测。
const STAGE_TEST_TIMEOUT = 5_460_000;
const STAGE_BUDGET_MS = 5_400_000;

const bootstrap = bootstrapRealJourney();

/** 驱动会话到定稿(纯 codex+三通道),然后 durable 断言+证据收口。 */
async function driveStoryToConfirm(bootstrap: JourneyBootstrap, session: SessionWorkspacePage, page?: Page): Promise<void> {
  await session.ensureCodexProviders(`${bootstrap.evidenceRoot}/s4-session`, "s4");
  const events = await session.driveUntilQuiet({
    stage: "s4",
    budgetMs: STAGE_BUDGET_MS,
    evidenceDir: `${bootstrap.evidenceRoot}/s4-session`,
    onHeartbeat: throttledConsole("s4"),
  });
  test.info().annotations.push({
    type: "s4-events",
    description: `gates=${events.gatesConfirmed.length}(${events.gatesConfirmed.map((gate) => gate.context.slice(0, 40)).join(";")}) choices=${events.choicesAnswered.length}`,
  });
  await assertStoryConfirmed(bootstrap);
  if (page) {
    await screenshotOf(bootstrap, page, "s4-story-confirmed");
    await dumpStage(bootstrap, "s4");
    expectNoDrift(bootstrap, boundarySnapshot(bootstrap, "after-s4"), "s4");
  }
}

/**
 * capability 墙的真实用户通路(add-provider-revalidate-probe,新层#2 收口):
 * 墙→点→等待→秒回→重试生成。前置版:S3 后/S4 前,工作台 LC 运维面板 →
 * provider capability 卡 → 对 codex 显式点击「核验」。
 * - 首击=真实边界探针(三 action × 真 CLI 轮次,分钟级有界等待;journey
 *   重放时 durable 已 Confirmed → 秒回 already_confirmed,两态皆接受);
 * - 成功后三行状态经真实 GET 刷新为「已核验」(不乐观改状态);
 * - 第二击断言秒回 already_confirmed(版本钉定幂等,仅一次 CLI --version,
 *   零探针会话)——首/次时长差即幂等证据,无 sleep;
 * - 失败如实抛出:墙仍在时生成必被 admission fail-closed 拒绝,不掩盖。
 */
async function revalidateCodexCapability(
  workbench: WorkbenchPage,
  page: Page,
): Promise<void> {
  await workbench.expandLogicalCodebasePanel();
  const card = page.getByTestId("provider-capability-card");
  await expect(card).toBeVisible({ timeout: 30_000 });
  const codex = card.getByTestId("provider-capability-provider-codex");
  await expect(codex).toBeVisible();
  const revalidateButton = codex.getByTestId(
    "provider-capability-revalidate-codex",
  );
  await expect(revalidateButton).toBeEnabled();

  const firstClickStartedAt = Date.now();
  await revalidateButton.click();
  const result = codex.getByTestId("provider-capability-result-codex");
  await expect(result).toContainText(/(revalidated|already_confirmed)/, {
    timeout: 330_000,
  });
  const firstClickElapsedMs = Date.now() - firstClickStartedAt;
  await expect(codex.getByTestId("provider-capability-codex-launch")).toHaveText(
    "已核验",
    { timeout: 30_000 },
  );

  const secondClickStartedAt = Date.now();
  await revalidateButton.click();
  await expect(result).toContainText("already_confirmed", { timeout: 60_000 });
  const secondClickElapsedMs = Date.now() - secondClickStartedAt;
  if (firstClickElapsedMs > 60_000) {
    expect(
      secondClickElapsedMs,
      "第二次核验应秒回(版本钉定幂等,零探针)",
    ).toBeLessThan(60_000);
  }
  test.info().annotations.push({
    type: "s4-capability-revalidate",
    description: `codex 首击 ${Math.round(firstClickElapsedMs / 1000)}s / 第二击 ${Math.round(secondClickElapsedMs / 1000)}s`,
  });
}

/** durable 双面:story spec confirmed + current_version 非空(定稿时落版本)。 */
async function assertStoryConfirmed(bootstrap: JourneyBootstrap): Promise<void> {
  await expect
    .poll(
      async () => {
        const lifecycle = await readIssueLifecycle(bootstrap);
        return { count: lifecycle.story_specs.length, status: lifecycle.story_specs[0]?.confirmation_status ?? "none" };
      },
      { timeout: 600_000, intervals: [5_000] },
    )
    .toEqual({ count: 1, status: "confirmed" });
  const lifecycle = await readIssueLifecycle(bootstrap);
  const story = lifecycle.story_specs[0]!;
  expect(story.current_version, "story current_version 应在定稿时写入").not.toBeNull();
  recordObserved(bootstrap.runId, { storySpecId: story.story_spec_id });
}

test.describe("real 全旅程 S4 Story", () => {
  test.setTimeout(STAGE_TEST_TIMEOUT);

  test("S4 story 生成与确认", async ({ page }) => {
    requirePreviousCleared(bootstrap, "s4");
    await runStage(bootstrap, "s4", async () => {
      const manifest = readManifest(bootstrap.runId);
      const issueTitle = manifest.observed.issueTitle;
      if (!issueTitle) throw new Error("台账缺少 issueTitle(S3 前置未完成)");

      // 阶段化重放:台账已记录 story 会话且未 confirmed → 直接重进该会话,
      // 不重复「生成」(避免第二个 story spec);否则走真实生成点击。
      const existingSessionId = manifest.observed.storySessionId ?? null;
      const existingStory = (await readIssueLifecycle(bootstrap).catch(() => null))?.story_specs.at(0) ?? null;
      let sessionId: string;
      if (existingSessionId && existingStory && existingStory.confirmation_status !== "confirmed") {
        sessionId = existingSessionId;
        test.info().annotations.push({ type: "s4-resume", description: `重进既有 story 会话 ${sessionId}` });
        const session = new SessionWorkspacePage(page, sessionId);
        await session.open();
        await session.ensureCodexProviders(`${bootstrap.evidenceRoot}/s4-session`, "s4");
        await driveStoryToConfirm(bootstrap, session, page);
        return;
      }
      if (existingStory?.confirmation_status === "confirmed") {
        test.info().annotations.push({ type: "s4-resume", description: "story 已 confirmed,仅复核 durable" });
        await assertStoryConfirmed(bootstrap);
        return;
      }

      const workbench = await enterWorkbench(bootstrap, page);

      // capability 墙前置(新层#2 收口):S3 后/S4 前经真实用户通路对 codex
      // 显式核验(首击真实探针有界等待,第二击秒回幂等断言),生成点击
      // 即免墙;resume/confirmed 重放路径不重复——capability 已在本
      // journey 早前 pass 中 Confirmed(否则会话无法推进到需重放的状态)。
      await revalidateCodexCapability(workbench, page);
      const generateButton = page.getByRole("button", { name: `生成 Story Spec ${issueTitle}` });
      await expect(generateButton).toBeVisible({ timeout: 30_000 });
      await expect(generateButton).toBeEnabled();
      await generateButton.click();

      // 响应携带 workspace_session 后自动导航;未导航=生成失败,捕获页面
      // 错误横幅(如 product store 错误)原文落证,不空等超时。
      try {
        await expect(page).toHaveURL(/\/workbench\/workspace\/[^/]+$/, { timeout: 120_000 });
      } catch {
        const banners = await page.getByRole("alert").allInnerTexts().catch(() => []);
        await screenshotOf(bootstrap, page, "s4-no-navigation");
        throw new Error(
          `生成 Story Spec 后未导航到会话页;页面错误横幅:${banners.join(" | ") || "(无 alert)"};URL=${page.url()}`,
        );
      }
      sessionId = page.url().split("/").pop()!;
      recordObserved(bootstrap.runId, { storySessionId: sessionId });
      test.info().annotations.push({ type: "s4-session", description: `storySessionId=${sessionId}` });

      const session = new SessionWorkspacePage(page, sessionId);
      await expect(session.shell).toBeVisible({ timeout: 60_000 });
      await driveStoryToConfirm(bootstrap, session, page);
    });
  });
});
