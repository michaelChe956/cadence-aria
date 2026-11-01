import { expect, type Locator, type Page } from "@playwright/test";

/// 工作台页面对象(/workbench,AppShell→IssueLifecycleWorkbench)。
/// 锚定纪律(v2.0 Q4):data-testid 定位容器、role/label 定位控件;
/// 禁 .first()/.last() 掩盖歧义;禁裸 sleep,expect 自动等待。

export class WorkbenchPage {
  readonly page: Page;

  constructor(page: Page) {
    this.page = page;
  }

  get shell(): Locator {
    return this.page.getByTestId("workbench-shell");
  }

  /** S0 前置断言:产品页面已渲染(非空 HTML/provider 错误)。 */
  async expectLoaded(): Promise<void> {
    await expect(this.shell).toBeVisible();
    await expect(this.page.getByRole("heading", { name: "Issue 生命周期工作台" })).toBeVisible();
  }

  /** 首次进入的真实提示:版本更新说明与操作引导,经真实点击关闭(不注入存储伪造已读)。 */
  async dismissFirstRunOverlays(): Promise<void> {
    const whatsNew = this.page.getByRole("dialog", { name: "版本更新说明" });
    if (await whatsNew.isVisible()) {
      await whatsNew.getByRole("button", { name: "关闭" }).click();
      await expect(whatsNew).toBeHidden();
    }
    const onboarding = this.page.getByTestId("onboarding-wizard");
    if (await onboarding.isVisible()) {
      await this.page.getByTestId("onboarding-skip").click();
      await expect(onboarding).toBeHidden();
    }
  }

  async createProject(name: string, description: string): Promise<void> {
    await this.page.getByTestId("onboarding-anchor-project-create").click();
    const dialog = this.page.getByRole("dialog", { name: "新建 Project" });
    await expect(dialog).toBeVisible();
    await dialog.getByLabel("Project 名称").fill(name);
    await dialog.getByLabel("Project 描述").fill(description);
    await dialog.getByRole("button", { name: "创建 Project" }).click();
    await expect(dialog).toBeHidden();
    const sidebarEntry = this.page
      .getByRole("navigation", { name: "Project 切换" })
      .getByRole("button", { name, exact: true });
    await expect(sidebarEntry).toBeVisible();
    await expect(sidebarEntry).toHaveAttribute("aria-pressed", "true");
  }

  async openAddCodebaseDialog(): Promise<Locator> {
    await this.page.getByTestId("onboarding-anchor-codebase-add").click();
    const dialog = this.page.getByRole("dialog", { name: "添加代码库" });
    await expect(dialog).toBeVisible();
    return dialog;
  }

  async expandLogicalCodebasePanel(): Promise<void> {
    const toggle = this.page.getByTestId("lc-summary-toggle");
    await expect(toggle).toBeVisible();
    if ((await toggle.getAttribute("aria-expanded")) !== "true") {
      await toggle.click();
    }
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
  }

  /** 阶段面板切换:单阶段工作区同一时刻只渲染当前 stage 的卡,而默认
   * 阶段=「需要动作的最早阶段」(无 Story→story;有 Story 无 Design→design;
   * 其余→work_item),进入工作台时未必停在目标卡所在 stage——取卡前先切。 */
  async openStage(stage: "story" | "design" | "work_item"): Promise<void> {
    const tab = this.page.getByTestId(`stage-tab-${stage}`);
    await expect(tab).toBeVisible({ timeout: 30_000 });
    await tab.click();
    await expect(this.page.locator(`#stage-panel-${stage}`)).toBeVisible();
  }

  get initializationStatus(): Locator {
    return this.page.getByTestId("aggregate-initialization-status");
  }

  async startAggregateInitialization(): Promise<void> {
    const card = this.page.getByTestId("aggregate-initialization-card");
    await expect(card).toBeVisible();
    await card.getByRole("button", { name: "启动聚合初始化" }).click();
  }

  initializationStep(stepId: string): Locator {
    return this.page.getByTestId(`aggregate-initialization-step-${stepId}`);
  }

  async openCreateIssueDialog(): Promise<Locator> {
    const entry = this.page.getByTestId("onboarding-anchor-issue-create");
    await expect(entry).toBeEnabled();
    await entry.click();
    const dialog = this.page.getByRole("dialog", { name: "新建 Issue" });
    await expect(dialog).toBeVisible();
    return dialog;
  }

  issueQueueRow(title: string): Locator {
    return this.page.getByRole("button", { name: `选择 Issue ${title}` });
  }

  async expectIssueVisible(title: string): Promise<void> {
    await expect(this.issueQueueRow(title)).toBeVisible({ timeout: 30_000 });
    await expect(
      this.page.getByRole("heading", { name: title, exact: true }),
    ).toBeVisible({ timeout: 30_000 });
  }
}
