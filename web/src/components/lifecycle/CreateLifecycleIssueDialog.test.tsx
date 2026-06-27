import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { CreateLifecycleIssueDialog } from "./CreateLifecycleIssueDialog";
import { deferred, repositoryRecord } from "./IssueLifecycleWorkbench.test-utils";

describe("CreateLifecycleIssueDialog", () => {
  it("shows submit errors and prevents duplicate submissions while pending", async () => {
    const submit = deferred<void>();
    const onCreate = vi.fn(() => submit.promise);
    const user = userEvent.setup();

    render(
      <CreateLifecycleIssueDialog
        repositories={[repositoryRecord()]}
        onCreate={onCreate}
        onClose={vi.fn()}
      />,
    );

    await user.type(screen.getByLabelText("Issue 标题"), "新增安全提示");
    await user.selectOptions(
      screen.getByLabelText("代码库"),
      "repository_0001",
    );
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));
    await user.click(screen.getByRole("button", { name: "创建 Issue" }));

    expect(onCreate).toHaveBeenCalledTimes(1);

    submit.reject(new Error("create issue failed"));
    expect(await screen.findByText("create issue failed")).toBeInTheDocument();
  });
});
