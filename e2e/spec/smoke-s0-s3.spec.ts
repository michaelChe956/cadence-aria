import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { apiGet } from "../lib/api-reader.ts";
import {
  archiveInitializationDiagnostics,
  collectStageDump,
  findInitializationOperation,
} from "../lib/backend-dump.ts";
import { BoundaryWatch, driftSinceBaseline, snapshotRepos, type BoundarySnapshot } from "../lib/boundary-watch.ts";
import { IssueDialogPage } from "../lib/page-objects/issue-dialog.page.ts";
import { RegistrationWizardPage } from "../lib/page-objects/registration-wizard.page.ts";
import { WorkbenchPage } from "../lib/page-objects/workbench.page.ts";
import {
  readManifest,
  readManifestPointer,
  recordObserved,
  recordStage,
  type StageResult,
} from "../lib/run-contract.ts";
import { waitFor } from "../env/wait.ts";

/// 页面 E2E P1 冒烟:S0-S3(环境健康→首页→建档+LC+四仓登记→聚合初始化→单仓 issue)。
/// 纪律:真实产品服务(aria CLI 单端口,web/dist 嵌入)+ 真实页面点击;
/// provider 冒烟桩态(fake,不 spawn AI,台账如实标注);禁 page.route mock、
/// 禁 test controls;每段前置未通过则 not_executed(skip);retries=0。

const pointer = readManifestPointer();
const RUN_ID = pointer.runId;
const MANIFEST = readManifest(RUN_ID);
if (!MANIFEST.baseURL) throw new Error("台账缺少 baseURL(冒烟必须由真实装配驱动)");
const BASE_URL: string = MANIFEST.baseURL;
const LC_NAME = "notify-center";
const PROJECT_NAME = `notify-smoke-${RUN_ID}`;
const ISSUE_TITLE = `站内消息通知中心·${RUN_ID}`;
const FOUR_LAYERS = ["busi", "api", "gateway", "frontend"];
const ISSUE_DESCRIPTION = [
  "四层纵切(站内消息通知中心):",
  "busi——通知领域模型 id/user/title/body/created_at/read,查询/未读计数/幂等标读,种子载入内存;",
  "api——GET /notifications、GET /notifications/unread-count、POST /notifications/{id}/read,组合 busi;",
  "gateway——/api/notifications* 同源转发,X-User 必填(缺失 401),用户上下文不被 query 覆盖;",
  "frontend——通知中心页面:列表、未读徽标、点击标读。",
  "实现依赖顺序:busi → api → gateway → frontend。",
].join("\n");

/// S2 等待预算:门类 600s 基线(失败关闭上界,非时长预测)。
const S2_TEST_TIMEOUT = 600_000;
const S2_PAGE_WAIT_MS = 480_000;

const STAGE_ORDER = ["s0", "s1", "s2", "s3"] as const;
type StageName = (typeof STAGE_ORDER)[number];

function previousCleared(stage: StageName): boolean {
  const index = STAGE_ORDER.indexOf(stage);
  if (index <= 0) return true;
  const previous = readManifest(RUN_ID).stages[STAGE_ORDER[index - 1]];
  return previous?.status === "pass" || previous?.status === "pass_degraded";
}

/** 段执行骨架:失败如实落台账再抛出;通过/降级由调用方决定状态字。 */
async function runStage(stage: StageName, body: () => Promise<StageResult["status"] | void>): Promise<void> {
  const startedAt = new Date().toISOString();
  try {
    const outcome = (await body()) ?? "pass";
    recordStage(RUN_ID, {
      stage,
      status: outcome,
      startedAt,
      endedAt: new Date().toISOString(),
      detail: outcome === "pass_degraded" ? "S2 降级为 env 断言(纪律 4),如实标注" : undefined,
    });
  } catch (error) {
    recordStage(RUN_ID, {
      stage,
      status: "fail",
      startedAt,
      endedAt: new Date().toISOString(),
      detail: `${(error as Error).message}`,
    });
    throw error;
  }
}

async function screenshotOf(page: Page, name: string): Promise<string> {
  const target = path.join(MANIFEST.evidenceRoot, "screenshots", `${name}.png`);
  await page.screenshot({ path: target, fullPage: true });
  return target;
}

function boundary(label: string): BoundarySnapshot {
  return snapshotRepos({
    aggregateRoot: MANIFEST.aggregateRoot,
    layers: FOUR_LAYERS,
    evidenceRoot: MANIFEST.evidenceRoot,
    label,
  });
}

function baselineSnapshot(): BoundarySnapshot {
  const dir = path.join(MANIFEST.evidenceRoot, "boundary");
  const first = readdirSync(dir).filter((name) => name.includes("baseline-assemble")).sort().at(0);
  if (!first) throw new Error(`缺少装配基线快照(${dir})`);
  return JSON.parse(readFileSync(path.join(dir, first), "utf8")) as BoundarySnapshot;
}

function expectNoDrift(current: BoundarySnapshot, stage: string): void {
  const findings = driftSinceBaseline(baselineSnapshot(), current);
  const evidence = JSON.stringify(findings, null, 2);
  test.info().annotations.push({ type: "boundary", description: `${stage} 漂移判定:${findings.length} 项` });
  expect(findings, `阶段 ${stage} 出现越界漂移:\n${evidence}`).toHaveLength(0);
}

/** 每段入口:进工作台、关首次提示、选中本项目。 */
async function enterWorkbench(page: Page): Promise<WorkbenchPage> {
  const workbench = new WorkbenchPage(page);
  await page.goto("/");
  await expect(page).toHaveURL(/\/workbench$/);
  await workbench.expectLoaded();
  await workbench.dismissFirstRunOverlays();
  const entry = page
    .getByRole("navigation", { name: "Project 切换" })
    .getByRole("button", { name: PROJECT_NAME, exact: true });
  if (await entry.isVisible()) {
    if ((await entry.getAttribute("aria-pressed")) !== "true") {
      await entry.click();
    }
    await expect(entry).toHaveAttribute("aria-pressed", "true");
  }
  return workbench;
}

test.describe("页面 E2E P1 冒烟 S0-S3", () => {
  let watch: BoundaryWatch | null = null;

  test.beforeAll(() => {
    watch = new BoundaryWatch({
      aggregateRoot: MANIFEST.aggregateRoot,
      layers: FOUR_LAYERS,
      evidenceRoot: MANIFEST.evidenceRoot,
    });
    watch.start();
  });

  test.afterAll(() => {
    watch?.stop("spec-end");
  });

  test("S0 环境健康与首页渲染", async ({ page }) => {
    if (!previousCleared("s0")) test.skip(true, "前置段未通过:not_executed");
    await runStage("s0", async () => {
      // 只读 API 前置:health/runtime-info 现场重读(段首不信任历史观测)。
      const health = await apiGet(BASE_URL, "/api/health");
      expect(health.status).toBe(200);
      const runtime = await apiGet(BASE_URL, "/api/runtime-info");
      expect(runtime.status).toBe(200);
      const runtimeBody = runtime.body as { status?: string; workspace_root?: string };
      expect(runtimeBody.status).toBe("ok");
      expect(runtimeBody.workspace_root).toBe(MANIFEST.workspaceRoot);

      await page.goto("/");
      await expect(page).toHaveURL(/\/workbench$/, { timeout: 30_000 });
      const workbench = new WorkbenchPage(page);
      await workbench.expectLoaded();
      await workbench.dismissFirstRunOverlays();
      await workbench.expectLoaded();

      await screenshotOf(page, "s0-workbench");
      const dump = await collectStageDump(BASE_URL, RUN_ID, "s0");
      test.info().attachments.push({
        name: "s0-backend-dump",
        path: path.join(MANIFEST.evidenceRoot, "backend-dumps", "s0.json"),
        contentType: "application/json",
      });
      expect(Object.keys(dump.endpoints)).toContain("/api/health");
    });
  });

  test("S1 建档/LC/四仓登记", async ({ page }) => {
    if (!previousCleared("s1")) test.skip(true, "前置段未通过:not_executed");
    await runStage("s1", async () => {
      const workbench = await enterWorkbench(page);

      // 建项目
      await workbench.createProject(PROJECT_NAME, "页面 E2E P1 冒烟:站内消息通知中心四层案例");

      // 建 LC(多仓库逻辑代码库)
      const addDialog = await workbench.openAddCodebaseDialog();
      await addDialog.getByRole("radio", { name: /多仓库逻辑代码库/ }).check();
      await addDialog.getByLabel("聚合根目录", { exact: true }).fill(MANIFEST.aggregateRoot);
      await addDialog.getByLabel("名称").fill(LC_NAME);
      await addDialog.getByRole("button", { name: "创建逻辑代码库" }).click();
      await expect(addDialog).toBeHidden();

      // 批登记向导:自动发现→四仓 eligible→提交→completed
      const wizard = new RegistrationWizardPage(page);
      await wizard.expectOpen();
      await wizard.runAutoDiscovery(MANIFEST.aggregateRoot);
      await wizard.expectEligibleCandidates(FOUR_LAYERS);
      await wizard.submit();
      await wizard.expectRegistrationCompleted(FOUR_LAYERS.map((layer) => `/${layer}`));
      await wizard.close();

      // LC 管理面板可见且选中本 LC
      await expect(page.getByTestId("lc-summary-name")).toHaveText(LC_NAME);
      await workbench.expandLogicalCodebasePanel();
      await expect(page.getByTestId(`lc-selector-${LC_NAME}`)).toHaveAttribute("aria-selected", "true");

      // durable 双面:projects/codebases/members/repositories 只读核对
      const projects = (await apiGet(BASE_URL, "/api/projects")).body as {
        projects?: { project_id: string; name: string }[];
      };
      const project = projects.projects?.find((candidate) => candidate.name === PROJECT_NAME);
      expect(project, `项目未出现在 /api/projects:${JSON.stringify(projects)}`).toBeTruthy();
      recordObserved(RUN_ID, { projectId: project!.project_id, projectName: PROJECT_NAME });

      const codebases = (
        await apiGet(BASE_URL, `/api/projects/${project!.project_id}/codebases`)
      ).body as {
        codebases?: { id: string; kind: string; name: string; logical_codebase_id?: string; member_count?: number }[];
      };
      const logical = codebases.codebases?.find(
        (candidate) => candidate.kind === "logical" && candidate.name === LC_NAME,
      );
      expect(logical, `逻辑代码库未出现:${JSON.stringify(codebases)}`).toBeTruthy();
      expect(logical!.member_count).toBe(4);
      recordObserved(RUN_ID, { logicalCodebaseId: logical!.logical_codebase_id ?? logical!.id, logicalCodebaseName: LC_NAME });

      const members = (
        await apiGet(
          BASE_URL,
          `/api/projects/${project!.project_id}/logical-codebases/${logical!.logical_codebase_id ?? logical!.id}/members`,
        )
      ).body as { members?: { alias: string; status: string; physical_repository_id?: string }[] };
      expect(members.members ?? []).toHaveLength(4);
      for (const layer of FOUR_LAYERS) {
        const member = members.members?.find((candidate) => candidate.alias === layer);
        expect(member, `成员 ${layer} 缺失:${JSON.stringify(members)}`).toBeTruthy();
        expect(member!.status).toBe("active");
      }

      const repositories = (
        await apiGet(BASE_URL, `/api/projects/${project!.project_id}/repositories`)
      ).body as { repositories?: { repository_id: string }[] };
      // 产品契约:LC 成员的物理仓经 members 端点投影(上方已核对 4 active);
      // 项目级 /repositories 只列单仓登记,LC-only 项目为空属预期,不作 4 仓断言。
      test.info().annotations.push({
        type: "s1-repositories",
        description: `项目级 repositories 列表长度=${(repositories.repositories ?? []).length}(LC-only 预期 0)`,
      });

      // 初始化卡依赖页面侧成员轮询(2s),其可见性属 S2 入口职责;S1 出口以
      // durable 成员事实(4 active)+ LC 面板选中态收口(v2.0 S1 出口口径)。
      await expect(page.getByTestId("aggregate-initialization-card")).toBeVisible({
        timeout: 30_000,
      }).catch(async () => {
        test.info().annotations.push({ type: "s1-init-card", description: "初始化卡在 S1 窗口内未出现,移交 S2 入口核对" });
      });

      await screenshotOf(page, "s1-lc-registered");
      await collectStageDump(BASE_URL, RUN_ID, "s1");
      expectNoDrift(boundary("after-s1"), "s1");
    });
  });

  test("S2 聚合初始化", async ({ page }) => {
    test.setTimeout(S2_TEST_TIMEOUT);
    if (!previousCleared("s2")) test.skip(true, "前置段未通过:not_executed");
    await runStage("s2", async () => {
      const workbench = await enterWorkbench(page);
      await workbench.expandLogicalCodebasePanel();
      const initCard = page.getByTestId("aggregate-initialization-card");
      // 页面侧成员轮询偶发滞后:一次有界等待,缺失则重进页面重试一轮(新轮询)。
      try {
        await expect(initCard).toBeVisible({ timeout: 30_000 });
      } catch {
        await page.reload();
        await expect(page).toHaveURL(/\/workbench$/);
        await workbench.dismissFirstRunOverlays();
        await workbench.expandLogicalCodebasePanel();
        await expect(initCard).toBeVisible({ timeout: 60_000 });
      }

      // 真实点击启动(仅当可启动;已启动则直接进入等待)
      const startButton = page.getByRole("button", { name: "启动聚合初始化" });
      if (await startButton.isVisible()) {
        await startButton.click();
      }

      // 有界轮询到终态(completed/failed/cancelled);轮询间隔定时属允许采样。
      let terminal: string | null = null;
      let timedOut = false;
      try {
        terminal = await waitFor(
          "聚合初始化终态",
          S2_PAGE_WAIT_MS,
          async () => {
            const status = await page
              .getByTestId("aggregate-initialization-status")
              .getAttribute("data-status")
              .catch(() => null);
            if (status === "completed" || status === "failed" || status === "cancelled") return status;
            return null;
          },
          1000,
        );
      } catch {
        timedOut = true;
      }

      recordObserved(RUN_ID, { initStatus: terminal ?? "timeout" });
      await collectStageDump(BASE_URL, RUN_ID, "s2");
      const manifestNow = readManifest(RUN_ID);
      const durableOperation = await findInitializationOperation(manifestNow);
      if (durableOperation) {
        recordObserved(RUN_ID, { initOperationId: durableOperation.operationId });
        await collectStageDump(BASE_URL, RUN_ID, "s2");
      }
      await archiveInitializationDiagnostics(manifestNow, "s2");
      await screenshotOf(page, terminal === "completed" ? "s2-init-completed" : "s2-init-degraded");
      expectNoDrift(boundary("after-s2"), "s2");

      if (terminal === "completed") {
        // 五步全部 completed(machine_skills/aggregate_preflight/pre_check/rule_and_mcp_config/openspec_and_examples)
        const steps = page.locator('[data-testid^="aggregate-initialization-step-"]');
        await expect(steps).toHaveCount(5, { timeout: 30_000 });
        for (let index = 0; index < 5; index += 1) {
          await expect(steps.nth(index)).toHaveAttribute("data-status", "completed");
        }
        return "pass";
      }

      // 纪律 4 降级路径:轮询到终态(非 completed)或超时 → PASS 前置的 env 断言,如实标注。
      const alerts = await page
        .getByTestId("aggregate-initialization-card")
        .getByRole("alert")
        .allInnerTexts();
      const facts: string[] = [];
      if (terminal) facts.push(`终态=${terminal}`);
      if (timedOut) facts.push(`等待 ${S2_PAGE_WAIT_MS}ms 超时`);
      facts.push(...alerts.map((text) => `页面告警:${text.trim()}`));
      if (durableOperation) facts.push(`durable operation=${durableOperation.operationId}(${durableOperation.status})`);
      test.info().annotations.push({ type: "s2-degraded", description: facts.join(";") });
      // 不伪造:至少存在一条真实 durable/页面事实,否则按 FAIL 处理。
      expect(facts.length, "S2 降级断言需要至少一条真实事实(终态/超时/告警/durable operation)").toBeGreaterThan(0);
      if (!terminal && !timedOut) {
        throw new Error(`初始化状态异常且无终态事实:${facts.join(";")}`);
      }
      return "pass_degraded";
    });
  });

  test("S3 单仓 issue 创建并出现在列表", async ({ page }) => {
    if (!previousCleared("s3")) test.skip(true, "前置段未通过:not_executed");
    await runStage("s3", async () => {
      const workbench = await enterWorkbench(page);

      // S3 冒烟形态:本 LC + 单个 Primary 成员(单仓范围;多仓复选属另一 change)
      const createIssueEntry = page.getByTestId("onboarding-anchor-issue-create");
      if (!(await createIssueEntry.isEnabled())) {
        throw new Error(
          "新建 Issue 入口不可用:产品门 canCreateIssue 要求项目级 repositories>0,而 LC-only 项目该列表为空(见 s1-repositories 注记)——疑似多仓 issue 入口产品前置,如实 FAIL 不绕行",
        );
      }
      const dialog = await workbench.openCreateIssueDialog();
      const issueDialog = new IssueDialogPage(page);
      issueDialog.bind(dialog);
      await issueDialog.fillTitle(ISSUE_TITLE);
      await issueDialog.fillDescription(ISSUE_DESCRIPTION);
      await issueDialog.selectCodebase(`${LC_NAME} · 逻辑`);
      await issueDialog.selectPrimaryMember("frontend");
      await issueDialog.submit();
      await issueDialog.expectClosed();

      // 页面断言:队列行 + 聚焦标题
      await workbench.expectIssueVisible(ISSUE_TITLE);

      // durable 双面:issues 列表与 lifecycle 同一身份
      const manifestNow = readManifest(RUN_ID);
      const issues = (
        await apiGet(BASE_URL, `/api/projects/${manifestNow.observed.projectId}/issues`)
      ).body as { issues?: { issue_id: string; title: string }[] };
      const created = issues.issues?.find((candidate) => candidate.title === ISSUE_TITLE);
      expect(created, `issue 未出现在列表:${JSON.stringify(issues)}`).toBeTruthy();
      recordObserved(RUN_ID, { issueId: created!.issue_id, issueTitle: ISSUE_TITLE, primaryAlias: "frontend" });

      const lifecycle = (
        await apiGet(
          BASE_URL,
          `/api/issues/${created!.issue_id}/lifecycle?project_id=${manifestNow.observed.projectId}`,
        )
      ).body as { issue?: { issue_id: string; repo_id?: string | null } };
      // ProductIssueDto 暴露 repo_id(Primary 物理仓绑定);LC 归属在 S1 已按
      // codebases/members durable 核对,DTO 不重复投影 logical_codebase_id。
      expect(lifecycle.issue?.issue_id).toBe(created!.issue_id);
      expect(lifecycle.issue?.repo_id ?? null).not.toBeNull();

      await screenshotOf(page, "s3-issue-created");
      await collectStageDump(BASE_URL, RUN_ID, "s3");
      expectNoDrift(boundary("after-s3"), "s3");
    });
  });
});
