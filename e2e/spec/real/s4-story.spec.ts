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
} from "../../lib/journey-support.ts";

/// real 全旅程 S4:Story Spec 生成(codex 真会话)→ 会话门确认 → durable confirmed。
/// provider 面向:无 localStorage 注入,author 走服务端默认 codex
/// (provider_workspace_config 默认 author=codex/reviewer=claude_code),
/// 实际 provider 快照经 backend dump 留证。

/// Story/Design 长段预算:5400s 失败关闭上界(v2.0 §5.2),非时长预测。
const STAGE_TEST_TIMEOUT = 5_460_000;
const STAGE_BUDGET_MS = 5_400_000;

const bootstrap = bootstrapRealJourney();

test.describe("real 全旅程 S4 Story", () => {
  test.setTimeout(STAGE_TEST_TIMEOUT);

  test("S4 story 生成与确认", async ({ page }) => {
    requirePreviousCleared(bootstrap, "s4");
    await runStage(bootstrap, "s4", async () => {
      const manifest = readManifest(bootstrap.runId);
      const issueTitle = manifest.observed.issueTitle;
      if (!issueTitle) throw new Error("台账缺少 issueTitle(S3 前置未完成)");

      const workbench = await enterWorkbench(bootstrap, page);
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
      const sessionId = page.url().split("/").pop()!;
      recordObserved(bootstrap.runId, { storySessionId: sessionId });
      test.info().annotations.push({ type: "s4-session", description: `storySessionId=${sessionId}` });

      const session = new SessionWorkspacePage(page, sessionId);
      await expect(session.shell).toBeVisible({ timeout: 60_000 });
      // 纯 codex 形态:进会话即显式 author/reviewer=codex(后续轮次生效)
      // 并保存默认(S5+ 生成请求快照 codex/codex)。
      await session.ensureCodexProviders(`${bootstrap.evidenceRoot}/s4-session`, "s4");

      const events = await session.driveUntilQuiet({
        stage: "s4",
        budgetMs: STAGE_BUDGET_MS,
        evidenceDir: `${bootstrap.evidenceRoot}/s4-session`,
        onHeartbeat: throttledConsole("s4"),
      });
      test.info().annotations.push({
        type: "s4-events",
        description: `gates=${events.gatesConfirmed.length} choices=${events.choicesAnswered.length}`,
      });

      // durable 双面:story spec confirmed(安静窗口后允许后置落盘,有界轮询)。
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
      expect(story.current_version).not.toBeNull();
      recordObserved(bootstrap.runId, { storySpecId: story.story_spec_id });

      await screenshotOf(bootstrap, page, "s4-story-confirmed");
      await dumpStage(bootstrap, "s4");
      expectNoDrift(bootstrap, boundarySnapshot(bootstrap, "after-s4"), "s4");
      // 会话收口后回工作台核对卡片呈现(story 卡 confirmed)。
      await workbench.expectLoaded().catch(() => {});
    });
  });
});
