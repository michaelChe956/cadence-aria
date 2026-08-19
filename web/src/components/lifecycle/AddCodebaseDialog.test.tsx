import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AddCodebaseDialog } from "./AddCodebaseDialog";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

describe("AddCodebaseDialog", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("默认单仓库模式：选择单仓库后走既有添加对话框流程", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    const onChooseSingle = vi.fn();
    const onClose = vi.fn();
    const user = userEvent.setup();

    render(
      <AddCodebaseDialog
        projectId="project_0001"
        onChooseSingle={onChooseSingle}
        onCreatedLogical={vi.fn()}
        onClose={onClose}
      />,
    );

    expect(
      screen.getByRole("radio", { name: /单仓库/ }),
    ).toBeChecked();
    await user.click(screen.getByRole("button", { name: "继续添加单仓库" }));

    expect(onChooseSingle).toHaveBeenCalledTimes(1);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("多仓库模式：name 默认聚合根目录名，创建成功回调 LC", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse({
          id: "lc_0001",
          name: "monorepo",
          aggregate_root: "/repos/monorepo",
          created_at: "2026-08-19T00:00:00Z",
        });
      }),
    );
    const onCreatedLogical = vi.fn();
    const user = userEvent.setup();

    render(
      <AddCodebaseDialog
        projectId="project_0001"
        onChooseSingle={vi.fn()}
        onCreatedLogical={onCreatedLogical}
        onClose={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("radio", { name: /多仓库逻辑代码库/ }));
    await user.type(screen.getByLabelText("聚合根目录"), "/repos/monorepo");
    expect(screen.getByLabelText("名称")).toHaveValue("monorepo");
    await user.click(screen.getByRole("button", { name: "创建逻辑代码库" }));

    await vi.waitFor(() => expect(onCreatedLogical).toHaveBeenCalledTimes(1));
    expect(calls.map(({ input }) => input)).toEqual([
      "/api/projects/project_0001/logical-codebases",
    ]);
    expect(JSON.parse(String(calls[0].init?.body))).toEqual({
      name: "monorepo",
      aggregate_root: "/repos/monorepo",
    });
    expect(onCreatedLogical).toHaveBeenCalledWith(
      expect.objectContaining({ id: "lc_0001" }),
    );
  });

  it("name 手工输入后不被聚合根目录覆盖；创建失败展示错误", async () => {
    let fail = true;
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        if (fail) {
          return jsonResponse(
            { code: "logical_codebase_name_required", message: "name must not be empty", details: {} },
            422,
          );
        }
        return jsonResponse({});
      }),
    );
    const user = userEvent.setup();

    render(
      <AddCodebaseDialog
        projectId="project_0001"
        onChooseSingle={vi.fn()}
        onCreatedLogical={vi.fn()}
        onClose={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("radio", { name: /多仓库逻辑代码库/ }));
    await user.type(screen.getByLabelText("聚合根目录"), "/repos/monorepo");
    await user.clear(screen.getByLabelText("名称"));
    await user.type(screen.getByLabelText("名称"), "custom-name");
    await user.click(screen.getByRole("button", { name: "创建逻辑代码库" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "name must not be empty",
    );

    fail = false;
    await user.click(screen.getByRole("button", { name: "创建逻辑代码库" }));
    expect(screen.getByLabelText("名称")).toHaveValue("custom-name");
  });
});
