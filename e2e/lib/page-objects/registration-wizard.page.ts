import { expect, type Locator, type Page } from "@playwright/test";

/// LC 批登记向导页面对象(LogicalCodebaseRegistrationWizard)。
/// 旅程:填聚合根→自动发现预检→核对 eligible 候选(默认全选)→提交登记→
/// 登记结果 completed(v2.0 §8:不得走单仓路由(非 LC)的逐仓表单)。
///
/// 同一向导有两种宿主形态,共用同一套步骤方法,差异只在 root 容器:
/// - 独立弹窗(面板「登记成员」按钮,给既有 LC 追加登记):root 缺省=role=dialog「登记成员」,
///   聚合根需用户填,故先 runAutoDiscovery。
/// - 内嵌合并流(「添加代码库」弹窗创建多仓库 LC 后):root=宿主 role=dialog「添加代码库」,
///   组件带 initialAggregateRoot+autoStart,挂载即自动发现;用 expectEmbeddedReady 等候选就绪,
///   不得再调 runAutoDiscovery(聚合根已预填且输入在后端预检完成后即禁用)。

export class RegistrationWizardPage {
  /// 登记流程所在容器:独立弹窗或内嵌宿主弹窗(内嵌形态由调用方传入宿主对话框)。
  readonly root: Locator;

  constructor(page: Page, root?: Locator) {
    this.root = root ?? page.getByRole("dialog", { name: "登记成员" });
  }

  get dialog(): Locator {
    return this.root;
  }

  /// 独立弹窗形态:向导对话框已打开。
  async expectOpen(): Promise<void> {
    await expect(this.dialog).toBeVisible();
  }

  /// 内嵌形态:等待登记阶段就绪——自动发现在挂载时已触发,
  /// 「提交登记」可点即代表预检完成且 eligible 候选已默认全选。
  async expectEmbeddedReady(): Promise<void> {
    await expect(this.dialog.getByRole("button", { name: "提交登记" })).toBeEnabled({
      timeout: 60_000,
    });
  }

  /// 独立弹窗形态专用:手工填聚合根并触发自动发现;内嵌形态已自动发现,勿调。
  async runAutoDiscovery(aggregateRoot: string): Promise<void> {
    await this.dialog.getByLabel("聚合根目录").fill(aggregateRoot);
    await this.dialog
      .getByRole("button", { name: "确认聚合根并自动发现" })
      .click();
  }

  eligibleCheckboxes(): Locator {
    return this.dialog
      .locator('section[aria-label="分类 eligible"] input[type="checkbox"]');
  }

  /** 断言自动发现出的 eligible 候选数与全选默认;路径需含期望层名。 */
  async expectEligibleCandidates(layers: string[]): Promise<void> {
    const checkboxes = this.eligibleCheckboxes();
    await expect(checkboxes).toHaveCount(layers.length, { timeout: 30_000 });
    const labels = await this.dialog
      .locator('section[aria-label="分类 eligible"] label')
      .allInnerTexts();
    const joined = labels.join("\n");
    for (const layer of layers) {
      if (!joined.includes(`/${layer}`)) {
        throw new Error(`eligible 候选缺少 ${layer} 层:${labels.join(" | ")}`);
      }
    }
    for (const layer of layers) {
      await expect(
        this.dialog.locator(`section[aria-label="分类 eligible"] input[aria-label*="/${layer}"]`),
      ).toBeChecked();
    }
  }

  async submit(): Promise<void> {
    await this.dialog.getByRole("button", { name: "提交登记" }).click();
  }

  /** 登记结果:批状态 completed 且每个成员 item completed。 */
  async expectRegistrationCompleted(expectedPaths: string[]): Promise<void> {
    const result = this.root.getByRole("region", { name: "登记结果" });
    await expect(result).toBeVisible({ timeout: 60_000 });
    await expect(result.getByText("completed", { exact: true })).toBeVisible({ timeout: 60_000 });
    const itemLines = await result.locator("li").allInnerTexts();
    for (const pathFragment of expectedPaths) {
      const matched = itemLines.find((line) => line.includes(pathFragment) && line.includes("completed"));
      if (!matched) {
        throw new Error(
          `登记结果缺少 ${pathFragment}:completed;实际:${itemLines.join(" | ")}`,
        );
      }
    }
  }

  /// 关闭并断言容器消失:独立弹窗形态=向导头部「关闭」;
  /// 内嵌形态=宿主「添加代码库」弹窗头部「关闭」(root 即宿主弹窗)。
  async close(): Promise<void> {
    await this.dialog.getByRole("button", { name: "关闭" }).click();
    await expect(this.dialog).toBeHidden();
  }
}
