import { expect, test } from "@playwright/test";
import { apiGet } from "../../lib/api-reader.ts";
import { BoundaryWatch } from "../../lib/boundary-watch.ts";
import { archiveInitializationDiagnostics, findInitializationOperation } from "../../lib/backend-dump.ts";
import { IssueDialogPage } from "../../lib/page-objects/issue-dialog.page.ts";
import { RegistrationWizardPage } from "../../lib/page-objects/registration-wizard.page.ts";
import { WorkbenchPage } from "../../lib/page-objects/workbench.page.ts";
import { readManifest, recordObserved } from "../../lib/run-contract.ts";
import { waitFor } from "../../env/wait.ts";
import {
  FOUR_LAYERS,
  LC_NAME,
  bootstrapRealJourney,
  boundarySnapshot,
  dumpStage,
  enterWorkbench,
  expectNoDrift,
  requirePreviousCleared,
  runStage,
  screenshotOf,
} from "../../lib/journey-support.ts";

/// real 全旅程 S0-S3:与冒烟同款页面动作,provider=real(codex/claude 真 spawn):
/// S2 聚合初始化必须真五步 completed(7200s 失败关闭上界,非时长预测);
/// S3 issue 为四仓全勾形态(本 LC 四成员复选)。

const bootstrap = bootstrapRealJourney();
const RUN_ID = bootstrap.runId;
const MANIFEST = bootstrap.manifest;
const BASE_URL = bootstrap.baseURL;
const PROJECT_NAME = `notify-journey-${RUN_ID}`;
const ISSUE_TITLE = `站内消息通知中心·${RUN_ID}`;
const ISSUE_DESCRIPTION = [
  "四层纵切(站内消息通知中心):",
  "busi——通知领域模型 id/user/title/body/created_at/read,查询/未读计数/幂等标读,种子 JSON 载入内存;",
  "api——GET /notifications、GET /notifications/unread-count、POST /notifications/{id}/read,组合 busi;",
  "gateway——/api/notifications* 同源转发,X-User 必填(缺失 401),用户上下文不被 query 覆盖;",
  "frontend——通知中心页面:列表、未读徽标、点击标读。",
  "实现依赖顺序:busi → api → gateway → frontend(依赖提供者先做)。",
  "全部四仓(busi/api/gateway/frontend)都必须有真实业务代码变更。",
].join("\n");

/// 初始化真跑上界(E7 矩阵 7200s);测试超时留采样余量。
const S2_TEST_TIMEOUT = 7_500_000;
const S2_PAGE_WAIT_MS = 7_200_000;

test.describe("real 全旅程 S0-S3(建档/LC/四仓登记/真五步初始化/四仓 issue)", () => {
  let watch: BoundaryWatch | null = null;

  test.beforeAll(() => {
    watch = new BoundaryWatch({
      aggregateRoot: MANIFEST.aggregateRoot,
      layers: [...FOUR_LAYERS],
      evidenceRoot: MANIFEST.evidenceRoot,
    });
    watch.start();
  });

  test.afterAll(() => {
    watch?.stop("s0s3-real-end");
  });

  test("S0 环境健康与首页(real)", async ({ page }) => {
    requirePreviousCleared(bootstrap, "s0");
    await runStage(bootstrap, "s0", async () => {
      const health = await apiGet(BASE_URL, "/api/health");
      expect(health.status).toBe(200);
      const runtime = await apiGet(BASE_URL, "/api/runtime-info");
      expect(runtime.status).toBe(200);
      const runtimeBody = runtime.body as { status?: string; workspace_root?: string };
      expect(runtimeBody.status).toBe("ok");
      expect(runtimeBody.workspace_root).toBe(MANIFEST.workspaceRoot);
      // real 模式硬前置:codex health 预热事实在台账;服务侧 status 必须就绪。
      const providers = (await apiGet(BASE_URL, "/api/providers/status")).body as {
        state_status?: string;
        providers?: { provider: string; available: boolean }[];
      };
      expect(providers.state_status).toBe("ready");
      const codex = providers.providers?.find((entry) => entry.provider === "codex");
      expect(codex, `codex 不在 provider status:${JSON.stringify(providers)}`).toBeTruthy();
      expect(codex!.available).toBe(true);

      await page.goto("/");
      await expect(page).toHaveURL(/\/workbench$/, { timeout: 30_000 });
      const workbench = new WorkbenchPage(page);
      await workbench.expectLoaded();
      await workbench.dismissFirstRunOverlays();
      await workbench.expectLoaded();
      await screenshotOf(bootstrap, page, "s0-workbench-real");
      await dumpStage(bootstrap, "s0");
    });
  });

  test("S1 建档/LC/四仓登记(real)", async ({ page }) => {
    requirePreviousCleared(bootstrap, "s1");
    await runStage(bootstrap, "s1", async () => {
      const workbench = await enterWorkbench(bootstrap, page);
      await workbench.createProject(PROJECT_NAME, "页面 E2E real 全旅程:站内消息通知中心四层案例");

      const addDialog = await workbench.openAddCodebaseDialog();
      await addDialog.getByRole("radio", { name: /多仓库逻辑代码库/ }).check();
      await addDialog.getByLabel("聚合根目录", { exact: true }).fill(MANIFEST.aggregateRoot);
      await addDialog.getByLabel("名称").fill(LC_NAME);
      await addDialog.getByRole("button", { name: "创建逻辑代码库" }).click();
      await expect(addDialog).toBeHidden();

      const wizard = new RegistrationWizardPage(page);
      await wizard.expectOpen();
      await wizard.runAutoDiscovery(MANIFEST.aggregateRoot);
      await wizard.expectEligibleCandidates([...FOUR_LAYERS]);
      await wizard.submit();
      await wizard.expectRegistrationCompleted(FOUR_LAYERS.map((layer) => `/${layer}`));
      await wizard.close();

      await expect(page.getByTestId("lc-summary-name")).toHaveText(LC_NAME);
      await workbench.expandLogicalCodebasePanel();
      await expect(page.getByTestId(`lc-selector-${LC_NAME}`)).toHaveAttribute("aria-selected", "true");

      const projects = (await apiGet(BASE_URL, "/api/projects")).body as {
        projects?: { project_id: string; name: string }[];
      };
      const project = projects.projects?.find((candidate) => candidate.name === PROJECT_NAME);
      expect(project, `项目未出现:${JSON.stringify(projects)}`).toBeTruthy();
      recordObserved(RUN_ID, { projectId: project!.project_id, projectName: PROJECT_NAME });

      const codebases = (
        await apiGet(BASE_URL, `/api/projects/${project!.project_id}/codebases`)
      ).body as { codebases?: { id: string; kind: string; name: string; logical_codebase_id?: string; member_count?: number }[] };
      const logical = codebases.codebases?.find(
        (candidate) => candidate.kind === "logical" && candidate.name === LC_NAME,
      );
      expect(logical, `LC 未出现:${JSON.stringify(codebases)}`).toBeTruthy();
      expect(logical!.member_count).toBe(4);
      recordObserved(RUN_ID, {
        logicalCodebaseId: logical!.logical_codebase_id ?? logical!.id,
        logicalCodebaseName: LC_NAME,
      });

      const members = (
        await apiGet(
          BASE_URL,
          `/api/projects/${project!.project_id}/logical-codebases/${logical!.logical_codebase_id ?? logical!.id}/members`,
        )
      ).body as { members?: { alias: string; status: string }[] };
      expect(members.members ?? []).toHaveLength(4);
      for (const layer of FOUR_LAYERS) {
        const member = members.members?.find((candidate) => candidate.alias === layer);
        expect(member, `成员 ${layer} 缺失`).toBeTruthy();
        expect(member!.status).toBe("active");
      }

      await screenshotOf(bootstrap, page, "s1-lc-registered-real");
      await dumpStage(bootstrap, "s1");
      expectNoDrift(bootstrap, boundarySnapshot(bootstrap, "after-s1"), "s1");
    });
  });

  test("S2 聚合初始化真五步(real)", async ({ page }) => {
    test.setTimeout(S2_TEST_TIMEOUT);
    requirePreviousCleared(bootstrap, "s2");
    await runStage(bootstrap, "s2", async () => {
      const workbench = await enterWorkbench(bootstrap, page);
      await workbench.expandLogicalCodebasePanel();
      const initCard = page.getByTestId("aggregate-initialization-card");
      try {
        await expect(initCard).toBeVisible({ timeout: 30_000 });
      } catch {
        await page.reload();
        await expect(page).toHaveURL(/\/workbench$/);
        await workbench.dismissFirstRunOverlays();
        await workbench.expandLogicalCodebasePanel();
        await expect(initCard).toBeVisible({ timeout: 60_000 });
      }

      const startButton = page.getByRole("button", { name: "启动聚合初始化" });
      if (await startButton.isVisible()) {
        await startButton.click();
      }

      let terminal: string | null = null;
      try {
        terminal = await waitFor(
          "聚合初始化终态(真五步)",
          S2_PAGE_WAIT_MS,
          async () => {
            const status = await page
              .getByTestId("aggregate-initialization-status")
              .getAttribute("data-status")
              .catch(() => null);
            if (status === "completed" || status === "failed" || status === "cancelled") return status;
            return null;
          },
          5_000,
        );
      } catch {
        terminal = null;
      }

      recordObserved(RUN_ID, { initStatus: terminal ?? "timeout" });
      await dumpStage(bootstrap, "s2");
      const manifestNow = readManifest(RUN_ID);
      const durableOperation = await findInitializationOperation(manifestNow);
      if (durableOperation) {
        recordObserved(RUN_ID, { initOperationId: durableOperation.operationId });
      }
      await archiveInitializationDiagnostics(manifestNow, "s2-real");
      await screenshotOf(bootstrap, page, terminal === "completed" ? "s2-init-completed-real" : "s2-init-failed-real");
      expectNoDrift(bootstrap, boundarySnapshot(bootstrap, "after-s2"), "s2");

      // real 旅程口径:初始化必须真 completed;失败/超时=FAIL(证据已落)。
      expect(terminal, `真五步初始化未 completed(终态=${terminal ?? "timeout"});durable=${durableOperation?.status ?? "-"};诊断已归档`).toBe("completed");
      const steps = page.locator('[data-testid^="aggregate-initialization-step-"]');
      await expect(steps).toHaveCount(5, { timeout: 30_000 });
      for (let index = 0; index < 5; index += 1) {
        await expect(steps.nth(index)).toHaveAttribute("data-status", "completed");
      }

      // 聚合索引硬前置(索引构建随初始化 detached 异步):story 生成依赖
      // active index;失败(如 member coverage)在 S2 落证而非 S4 兜底猜测。
      // 锚定:detached 索引成功只置索引记录 Active,不写回 manifest.
      // active_aggregate_index_id(生产零写回,已核);正确投影=bootstrap
      // 的 planning_ready(types/logical_codebase_bootstrap.rs:65)。
      const manifestNow2 = readManifest(RUN_ID);
      await expect
        .poll(
          async () => {
            const detail = (
              await apiGet(
                BASE_URL,
                `/api/projects/${manifestNow2.observed.projectId}/logical-codebases/${manifestNow2.observed.logicalCodebaseId}/bootstrap`,
                15_000,
              )
            ).body as { planning_ready?: boolean };
            return detail.planning_ready === true;
          },
          { timeout: 300_000, intervals: [5_000] },
        )
        .toBe(true);
    });
  });

  test("S3 四仓全勾 issue(real)", async ({ page }) => {
    requirePreviousCleared(bootstrap, "s3");
    await runStage(bootstrap, "s3", async () => {
      const workbench = await enterWorkbench(bootstrap, page);

      const createIssueEntry = page.getByTestId("onboarding-anchor-issue-create");
      if (!(await createIssueEntry.isEnabled())) {
        throw new Error("新建 Issue 入口不可用(canCreateIssue 门):如实 FAIL 不绕行");
      }
      const dialog = await workbench.openCreateIssueDialog();
      const issueDialog = new IssueDialogPage(page);
      issueDialog.bind(dialog);
      await issueDialog.fillTitle(ISSUE_TITLE);
      await issueDialog.fillDescription(ISSUE_DESCRIPTION);
      await issueDialog.selectCodebase(`${LC_NAME} · 逻辑`);
      // 全旅程形态:四仓全勾(单成员是冒烟形态)。
      await issueDialog.selectMembers([...FOUR_LAYERS]);
      await issueDialog.submit();
      await issueDialog.expectClosed();

      await workbench.expectIssueVisible(ISSUE_TITLE);

      const manifestNow = readManifest(RUN_ID);
      const issues = (
        await apiGet(BASE_URL, `/api/projects/${manifestNow.observed.projectId}/issues`)
      ).body as { issues?: { issue_id: string; title: string }[] };
      const created = issues.issues?.find((candidate) => candidate.title === ISSUE_TITLE);
      expect(created, `issue 未出现:${JSON.stringify(issues)}`).toBeTruthy();
      recordObserved(RUN_ID, { issueId: created!.issue_id, issueTitle: ISSUE_TITLE, primaryAlias: "busi" });

      const lifecycle = (
        await apiGet(
          BASE_URL,
          `/api/issues/${created!.issue_id}/lifecycle?project_id=${manifestNow.observed.projectId}`,
        )
      ).body as { issue?: { issue_id: string; repo_id?: string | null } };
      expect(lifecycle.issue?.issue_id).toBe(created!.issue_id);
      // 多仓 LC issue:repo_id 投影缺失属预期(逻辑代码库归属经 S1 durable 核对)。
      test.info().annotations.push({
        type: "s3-issue-scope",
        description: `issue repo_id=${lifecycle.issue?.repo_id ?? "null"}(LC 多仓形态)`,
      });

      await screenshotOf(bootstrap, page, "s3-issue-created-real");
      await dumpStage(bootstrap, "s3");
      expectNoDrift(bootstrap, boundarySnapshot(bootstrap, "after-s3"), "s3");
    });
  });
});
