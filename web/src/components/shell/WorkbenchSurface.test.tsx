import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { WorkbenchSurface } from "./WorkbenchSurface";

describe("WorkbenchSurface", () => {
  it("renders stable high-density workbench slots", () => {
    render(
      <WorkbenchSurface
        header={<span>Header content</span>}
        statusBar={<span>Status content</span>}
        alert={<div>Recoverable error</div>}
        main={<section aria-label="Primary panel">Main content</section>}
        aside={<section aria-label="Inspector panel">Aside content</section>}
      />,
    );

    expect(screen.getByRole("banner")).toHaveTextContent("Header content");
    expect(screen.getByRole("status")).toHaveTextContent("Status content");
    expect(screen.getByRole("alert")).toHaveTextContent("Recoverable error");
    expect(screen.getByRole("main", { name: "工作台主区域" })).toBeInTheDocument();
    expect(
      within(screen.getByRole("main", { name: "工作台主区域" })).getByRole("region", {
        name: "Primary panel",
      }),
    ).toHaveTextContent("Main content");
    expect(screen.getByRole("complementary", { name: "工作台检查器" })).toHaveTextContent(
      "Aside content",
    );
  });

  it("renders without optional status alert and aside slots", () => {
    render(<WorkbenchSurface header={<span>Header</span>} main={<span>Main only</span>} />);

    expect(screen.getByRole("banner")).toHaveTextContent("Header");
    expect(screen.getByRole("main", { name: "工作台主区域" })).toHaveTextContent("Main only");
    expect(screen.queryByRole("complementary", { name: "工作台检查器" })).not.toBeInTheDocument();
  });
});
