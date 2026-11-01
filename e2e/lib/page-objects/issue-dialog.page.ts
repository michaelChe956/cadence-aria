import { expect, type Locator, type Page } from "@playwright/test";

/// 新建 Issue 对话框页面对象(CreateLifecycleIssueDialog)。
/// S3 冒烟形态:选本 LC + 单个 Primary 成员(单仓范围;多仓复选属另一
/// change,本线不实现、不越界)。

export class IssueDialogPage {
  readonly page: Page;

  constructor(page: Page) {
    this.page = page;
  }

  bind(dialog: Locator): void {
    this.dialogLocator = dialog;
  }

  private dialogLocator: Locator | null = null;

  get dialog(): Locator {
    if (this.dialogLocator === null) {
      throw new Error("IssueDialogPage 未绑定 dialog(先经 WorkbenchPage.openCreateIssueDialog)");
    }
    return this.dialogLocator;
  }

  async fillTitle(title: string): Promise<void> {
    await this.dialog.getByLabel("Issue 标题").fill(title);
  }

  async fillDescription(description: string): Promise<void> {
    await this.dialog.getByLabel("Issue 描述").fill(description);
  }

  /** 代码库下拉按 option 文本选择(`<名称> · 逻辑` / `<名称> · 单仓`)。 */
  async selectCodebase(optionLabel: string): Promise<void> {
    await this.dialog.getByLabel("代码库").selectOption({ label: optionLabel });
  }

  /** 成员复选组(add-multi-repo-issue-entry 后契约):按别名勾选成员。
   * 单选等价=仅勾一个(冒烟形态);多选=连续调用。 */
  async selectPrimaryMember(alias: string): Promise<void> {
    const box = this.dialog.getByRole("checkbox", { name: new RegExp(alias) });
    await expect(box).toBeEnabled({ timeout: 30_000 });
    await box.check();
  }

  /** 多选:按别名列表勾选多个成员(全旅程 S3+ 形态)。 */
  async selectMembers(aliases: string[]): Promise<void> {
    for (const alias of aliases) {
      await this.selectPrimaryMember(alias);
    }
  }

  async submit(): Promise<void> {
    await this.dialog.getByRole("button", { name: "创建 Issue" }).click();
  }

  async expectClosed(): Promise<void> {
    await expect(this.dialog).toBeHidden({ timeout: 30_000 });
  }

  /** 收集对话框内全部可见告警(分支/成员/提交错误),不掩盖歧义。 */
  async visibleAlerts(): Promise<string[]> {
    const alerts = this.dialog.getByRole("alert");
    const texts: string[] = [];
    const count = await alerts.count();
    for (let index = 0; index < count; index += 1) {
      const text = (await alerts.nth(index).innerText()).trim();
      if (text.length > 0) texts.push(text);
    }
    return texts;
  }
}
