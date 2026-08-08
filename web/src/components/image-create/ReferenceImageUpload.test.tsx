import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useImageCreateStore } from "../../state/image-create-store";
import { ReferenceImageUpload } from "./ReferenceImageUpload";

const originalState = useImageCreateStore.getState();

beforeEach(() => {
  useImageCreateStore.setState({
    ...originalState,
    params: {
      ...originalState.params,
      input_fidelity: null,
    },
    referenceImage: null,
    isBusy: false,
  });
  vi.stubGlobal("URL", {
    ...URL,
    createObjectURL: vi.fn(() => "blob:reference-preview"),
    revokeObjectURL: vi.fn(),
  });
});

describe("ReferenceImageUpload", () => {
  it("rejects unsupported MIME types", () => {
    render(<ReferenceImageUpload />);

    fireEvent.change(screen.getByLabelText("上传参考图"), {
      target: {
        files: [new File(["gif"], "reference.gif", { type: "image/gif" })],
      },
    });

    expect(screen.getByRole("alert")).toHaveTextContent("仅支持 PNG、JPEG、WebP");
    expect(useImageCreateStore.getState().referenceImage).toBeNull();
  });

  it("rejects files larger than 10MB", () => {
    render(<ReferenceImageUpload />);
    const file = new File([new Uint8Array(10 * 1024 * 1024 + 1)], "large.png", {
      type: "image/png",
    });

    fireEvent.change(screen.getByLabelText("上传参考图"), {
      target: { files: [file] },
    });

    expect(screen.getByRole("alert")).toHaveTextContent("不能超过 10MB");
    expect(useImageCreateStore.getState().referenceImage).toBeNull();
  });

  it("previews an accepted image and removes it", async () => {
    const user = userEvent.setup();
    render(<ReferenceImageUpload />);
    const file = new File(["png"], "reference.png", { type: "image/png" });

    await user.upload(screen.getByLabelText("上传参考图"), file);

    expect(useImageCreateStore.getState().referenceImage).toBe(file);
    expect(useImageCreateStore.getState().params.input_fidelity).toBe("low");
    expect(screen.getByRole("img", { name: "参考图预览" })).toHaveAttribute(
      "src",
      "blob:reference-preview",
    );
    expect(screen.getByText("reference.png")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "移除参考图" }));
    expect(useImageCreateStore.getState().referenceImage).toBeNull();
    expect(screen.queryByRole("img", { name: "参考图预览" })).not.toBeInTheDocument();
  });

  it("disables upload and removal controls while busy", () => {
    const file = new File(["png"], "reference.png", { type: "image/png" });
    useImageCreateStore.setState({ referenceImage: file, isBusy: true });

    const { rerender } = render(<ReferenceImageUpload />);

    expect(screen.getByLabelText("上传参考图")).toBeDisabled();
    expect(screen.getByRole("button", { name: "移除参考图" })).toBeDisabled();
    expect(screen.getByText("处理中不可修改参考图")).toBeInTheDocument();

    useImageCreateStore.setState({ referenceImage: null });
    rerender(<ReferenceImageUpload />);

    expect(screen.getByText("选择一张参考图").closest("label")).toHaveAttribute(
      "aria-disabled",
      "true",
    );
  });
});
