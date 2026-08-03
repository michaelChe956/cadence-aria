import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionRecord } from "../../api/types/image-create";
import { useImageCreateStore } from "../../state/image-create-store";
import { ParamsPanel } from "./ParamsPanel";

const originalState = useImageCreateStore.getState();

beforeEach(() => {
  useImageCreateStore.setState({
    ...originalState,
    params: {
      prompt: "画一张商务插图",
      size: "auto",
      quality: "auto",
      background: "auto",
      output_format: "png",
      input_fidelity: null,
    },
    referenceImage: null,
    isBusy: false,
  });
});

describe("ParamsPanel", () => {
  it("binds all parameter selects to setParams and only generates on button click", async () => {
    const user = userEvent.setup();
    const setParams = vi.fn();
    const generate = vi.fn().mockResolvedValue(undefined);
    useImageCreateStore.setState({
      setParams,
      generate,
      currentSession: sessionRecord(),
    });

    render(<ParamsPanel />);

    await user.selectOptions(screen.getByLabelText("尺寸"), "1536x1024");
    await user.selectOptions(screen.getByLabelText("质量"), "high");
    await user.selectOptions(screen.getByLabelText("背景"), "transparent");
    await user.selectOptions(screen.getByLabelText("输出格式"), "webp");

    expect(setParams).toHaveBeenCalledWith({ size: "1536x1024" });
    expect(setParams).toHaveBeenCalledWith({ quality: "high" });
    expect(setParams).toHaveBeenCalledWith({ background: "transparent" });
    expect(setParams).toHaveBeenCalledWith({ output_format: "webp" });
    expect(generate).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "生成图片" }));
    expect(generate).toHaveBeenCalledTimes(1);
  });

  it("handles a rejected generation promise at the click boundary", async () => {
    const rejected = Promise.reject(new Error("生成失败"));
    void rejected.catch(() => {});
    const catchSpy = vi.spyOn(rejected, "catch");
    useImageCreateStore.setState({
      generate: vi.fn(() => rejected),
      currentSession: sessionRecord(),
    });

    render(<ParamsPanel />);
    fireEvent.click(screen.getByRole("button", { name: "生成图片" }));

    await waitFor(() => expect(catchSpy).toHaveBeenCalledTimes(1));
  });

  it("hides fidelity without a reference and shows it when a reference exists", () => {
    const { rerender } = render(<ParamsPanel />);
    expect(screen.queryByLabelText("参考图保真度")).not.toBeInTheDocument();

    useImageCreateStore.getState().setReferenceImage(
      new File(["image"], "reference.png", { type: "image/png" }),
    );
    rerender(<ParamsPanel />);

    expect(screen.getByLabelText("参考图保真度")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("参考图保真度"), {
      target: { value: "high" },
    });
    expect(useImageCreateStore.getState().params.input_fidelity).toBe("high");
  });
});

function sessionRecord(): SessionRecord {
  return {
    session: {
      id: "session-1",
      provider_name: "claude_code",
      template: { preset: "ppt_business_illustration" },
      last_provider_session_id: null,
      current_prompt: "画一张商务插图",
      status: "active",
      created_at: "2026-08-03T10:00:00Z",
    },
    messages: [],
    prompt_blocks: [],
    generation_results: [],
    events: [],
    generation: 0,
  };
}
