// Task 9：「知识库更新」弹窗的测试。
// 测试名按 task-9-brief Step 1 逐字落位（update_dialog_*）。
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createKnowledgeBaseChangePackage,
  listKnowledgeBaseChangePackages,
} from "../../api/knowledge-base";
import { KnowledgeBaseUpdateDialog } from "./KnowledgeBaseUpdateDialog";
import type { KnowledgeBaseUpdateDialogProps } from "./KnowledgeBaseUpdateDialog";

vi.mock("../../api/knowledge-base", () => ({
  listKnowledgeBaseChangePackages: vi.fn(),
  createKnowledgeBaseChangePackage: vi.fn(),
}));

const listChangePackagesMock = vi.mocked(listKnowledgeBaseChangePackages);
const createChangePackageMock = vi.mocked(createKnowledgeBaseChangePackage);

const CLEAN_PACKAGE = {
  id: "cp_0001",
  path: "changes/add-login.md",
  violation_count: 0,
  violations: [],
};

const DIRTY_PACKAGE = {
  id: "cp_0002",
  path: "changes/add-provider.md",
  violation_count: 2,
  violations: [
    {
      code: "field_empty",
      document: "changes/add-provider.md",
      field: "scope",
      detail: "scope 字段未填写",
    },
    {
      code: "field_empty",
      document: "changes/add-provider.md",
      field: "impact",
      detail: "impact 字段未填写",
    },
  ],
};

function renderDialog(overrides: Partial<KnowledgeBaseUpdateDialogProps> = {}) {
  const props: KnowledgeBaseUpdateDialogProps = {
    projectId: "project_0001",
    logicalCodebaseId: "logical_codebase_0001",
    open: true,
    onClose: vi.fn(),
    onStart: vi.fn(),
    ...overrides,
  };
  const { container } = render(<KnowledgeBaseUpdateDialog {...props} />);
  return { props, container };
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

describe("KnowledgeBaseUpdateDialog", () => {
  it("update_dialog_lists_packages_with_violation_counts", async () => {
    listChangePackagesMock.mockResolvedValue([CLEAN_PACKAGE, DIRTY_PACKAGE]);
    renderDialog();

    const cleanRow = await screen.findByTestId(
      "knowledge-base-package-changes/add-login.md",
    );
    expect(cleanRow.textContent).toContain("问题数：0");

    const dirtyRow = screen.getByTestId(
      "knowledge-base-package-changes/add-provider.md",
    );
    expect(dirtyRow.textContent).toContain("问题数：2");

    // 选中带违规的包 → 「执行更新」禁用。
    fireEvent.click(within(dirtyRow).getByRole("radio"));
    expect(screen.getByRole("button", { name: "执行更新" })).toBeDisabled();

    // 换选门禁通过的包 → 可用。
    fireEvent.click(within(cleanRow).getByRole("radio"));
    expect(screen.getByRole("button", { name: "执行更新" })).toBeEnabled();
  });

  it("update_dialog_executes_when_lint_clean", async () => {
    listChangePackagesMock.mockResolvedValue([CLEAN_PACKAGE]);
    const onStart = vi.fn();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    renderDialog({ onStart });

    const row = await screen.findByTestId(
      "knowledge-base-package-changes/add-login.md",
    );
    fireEvent.click(within(row).getByRole("radio"));

    fireEvent.click(screen.getByRole("button", { name: "执行更新" }));
    // 未确认 → 不触发。
    expect(onStart).not.toHaveBeenCalled();

    // 确认文案必须写明三件事：真实调用 provider、消耗额度、写入聚合根目录。
    const confirmText = String(confirmSpy.mock.calls[0]?.[0] ?? "");
    expect(confirmText).toContain("真实调用 provider");
    expect(confirmText).toContain("消耗额度");
    expect(confirmText).toContain("写入聚合根目录");

    confirmSpy.mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: "执行更新" }));
    expect(onStart).toHaveBeenCalledWith("update", "changes/add-login.md");
  });

  it("update_dialog_creates_package_and_prompts_to_fill", async () => {
    const user = userEvent.setup();
    listChangePackagesMock.mockResolvedValue([]);
    createChangePackageMock.mockResolvedValue({
      id: "cp_new",
      path: "changes/new-feature.md",
    });
    renderDialog();

    await screen.findByTestId("knowledge-base-update-dialog");
    await user.type(
      screen.getByTestId("knowledge-base-new-package-identifier"),
      "new-feature",
    );
    await user.click(screen.getByRole("button", { name: "新建变更包" }));

    expect(createChangePackageMock).toHaveBeenCalledWith(
      "project_0001",
      "logical_codebase_0001",
      "new-feature",
    );
    // 成功后提示用户去详情页填写。
    const result = await screen.findByTestId("knowledge-base-create-package-result");
    expect(result.textContent).toContain("changes/new-feature.md");
    expect(result.textContent).toContain("知识库详情页");
  });

  it("update_dialog_hidden_when_not_open", () => {
    listChangePackagesMock.mockResolvedValue([CLEAN_PACKAGE]);
    renderDialog({ open: false });
    expect(
      screen.queryByTestId("knowledge-base-update-dialog"),
    ).not.toBeInTheDocument();
    expect(listChangePackagesMock).not.toHaveBeenCalled();
  });

  // 修复轮 1 回归：外壳必须复用 shared/DialogShell——宿主链（面板 → 管理抽屉，
  // 面板恒挂 translate-x-* transform）会劫持 fixed 包含块，自绘蒙层只盖抽屉列。
  // jsdom 测不了视觉几何，但可证伪结构契约：卡片经 portal 挂 document.body
  // （不在宿主容器内）、打开期间 body 滚动锁定、Esc 经外壳三路关闭只关本层。
  it("update_dialog_uses_shared_shell_portal", async () => {
    listChangePackagesMock.mockResolvedValue([]);
    const { props, container } = renderDialog();

    const card = await screen.findByTestId("knowledge-base-update-dialog");
    expect(container.contains(card)).toBe(false);
    expect(document.body.contains(card)).toBe(true);
    expect(document.body.style.overflow).toBe("hidden");

    fireEvent.keyDown(window, { key: "Escape" });
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });
});
