import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ModelMultiSelect } from "./ModelMultiSelect";

describe("ModelMultiSelect", () => {
  it("收起时显示已选数量", () => {
    render(
      <ModelMultiSelect
        candidates={["m-a", "m-b"]}
        selected={["m-a"]}
        onChange={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("button", { name: "已选 1 项" }),
    ).toBeInTheDocument();
  });

  it("展开后列出候选并可用复选框勾选", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <ModelMultiSelect
        candidates={["m-a", "m-b"]}
        selected={["m-a"]}
        onChange={onChange}
      />,
    );

    await user.click(screen.getByRole("button", { name: "已选 1 项" }));

    expect(screen.getByRole("checkbox", { name: "m-a" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "m-b" })).not.toBeChecked();

    await user.click(screen.getByRole("checkbox", { name: "m-b" }));

    expect(onChange).toHaveBeenCalledWith(["m-a", "m-b"]);
  });

  it("取消勾选时给出去掉该项的已选数组", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <ModelMultiSelect
        candidates={["m-a", "m-b"]}
        selected={["m-a", "m-b"]}
        onChange={onChange}
      />,
    );

    await user.click(screen.getByRole("button", { name: "已选 2 项" }));
    await user.click(screen.getByRole("checkbox", { name: "m-a" }));

    expect(onChange).toHaveBeenCalledWith(["m-b"]);
  });

  it("网关未列出的候选带「网关未列出」标记", async () => {
    const user = userEvent.setup();
    render(
      <ModelMultiSelect
        candidates={["m-a", "custom-x"]}
        selected={["m-a"]}
        notListed={["custom-x"]}
        onChange={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "已选 1 项" }));

    expect(screen.getByText("网关未列出")).toBeInTheDocument();
  });

  it("添加自定义模型：把 id 勾选上，重复 id 不重复添加", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <ModelMultiSelect
        candidates={["m-a"]}
        selected={["m-a"]}
        onChange={onChange}
      />,
    );

    await user.click(screen.getByRole("button", { name: "已选 1 项" }));
    const input = screen.getByLabelText("添加自定义模型");
    await user.type(input, "custom-x");
    await user.click(screen.getByRole("button", { name: "添加" }));

    expect(onChange).toHaveBeenCalledWith(["m-a", "custom-x"]);

    // 重复 id：已在候选池且已勾选时不再重复添加。
    onChange.mockClear();
    await user.clear(input);
    await user.type(input, "m-a");
    await user.click(screen.getByRole("button", { name: "添加" }));

    expect(onChange).not.toHaveBeenCalled();
  });

  it("空输入不生效", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <ModelMultiSelect
        candidates={["m-a"]}
        selected={["m-a"]}
        onChange={onChange}
      />,
    );

    await user.click(screen.getByRole("button", { name: "已选 1 项" }));
    await user.type(screen.getByLabelText("添加自定义模型"), "   ");
    await user.click(screen.getByRole("button", { name: "添加" }));

    expect(onChange).not.toHaveBeenCalled();
  });

  it("候选池为空时给出占位文案", async () => {
    const user = userEvent.setup();
    render(
      <ModelMultiSelect candidates={[]} selected={[]} onChange={vi.fn()} />,
    );

    await user.click(screen.getByRole("button", { name: "已选 0 项" }));

    expect(
      screen.getByText(
        "暂无模型，点「测试连通」可拉取网关支持的模型，或手动添加",
      ),
    ).toBeInTheDocument();
  });

  it("Esc 收起展开面板", async () => {
    const user = userEvent.setup();
    render(
      <ModelMultiSelect
        candidates={["m-a"]}
        selected={["m-a"]}
        onChange={vi.fn()}
      />,
    );

    const trigger = screen.getByRole("button", { name: "已选 1 项" });
    await user.click(trigger);
    expect(screen.getByRole("checkbox", { name: "m-a" })).toBeInTheDocument();

    await user.keyboard("{Escape}");

    expect(screen.queryByRole("checkbox", { name: "m-a" })).toBeNull();
    expect(trigger).toHaveFocus();
  });

  it("复选框可用空格键勾选", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <ModelMultiSelect
        candidates={["m-a", "m-b"]}
        selected={["m-a"]}
        onChange={onChange}
      />,
    );

    await user.click(screen.getByRole("button", { name: "已选 1 项" }));
    const checkbox = screen.getByRole("checkbox", { name: "m-b" });
    checkbox.focus();
    await user.keyboard(" ");

    expect(onChange).toHaveBeenCalledWith(["m-a", "m-b"]);
  });
});
