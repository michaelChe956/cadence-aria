import { expect, type Locator, type Page } from "@playwright/test";

/// LC 批登记向导页面对象(LogicalCodebaseRegistrationWizard)。
/// 旅程:填聚合根→自动发现预检→核对 eligible 候选(默认全选)→提交登记→
/// 登记结果 completed(v2.0 §8:不得走单仓路由(非 LC)的逐仓表单)。

export class RegistrationWizardPage {
  readonly page: Page;

  constructor(page: Page) {
    this.page = page;
  }

  get dialog(): Locator {
    return this.page.getByRole("dialog", { name: "登记成员" });
  }

  async expectOpen(): Promise<void> {
    await expect(this.dialog).toBeVisible();
  }

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
    const result = this.page.getByRole("region", { name: "登记结果" });
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

  async close(): Promise<void> {
    await this.dialog.getByRole("button", { name: "关闭" }).click();
    await expect(this.dialog).toBeHidden();
  }
}
