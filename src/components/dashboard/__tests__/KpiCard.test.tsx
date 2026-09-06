import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { Package } from "lucide-react";
import { KpiCard } from "@/components/dashboard/KpiCard";
import { Badge } from "@/components/ui/Badge";

describe("KpiCard", () => {
  it("renders label, value and sub content", () => {
    render(
      <KpiCard
        label="Total detectados"
        value="14"
        sub={<span>6.42 GB</span>}
        icon={Package}
      />,
    );

    expect(screen.getByText("Total detectados")).toBeInTheDocument();
    expect(screen.getByText("14")).toBeInTheDocument();
    expect(screen.getByText("6.42 GB")).toBeInTheDocument();
  });

  it("applies the tone's border class", () => {
    const { container } = render(
      <KpiCard label="Falhas detectadas" value={1} tone="error" icon={Package} />,
    );

    const card = container.firstElementChild;
    expect(card?.className).toContain("color-error");
  });

  it("renders a Badge sub slot for alert states", () => {
    render(
      <KpiCard
        label="Falhas detectadas"
        value={1}
        tone="error"
        sub={<Badge tone="error">{"Requer atenção"}</Badge>}
        icon={Package}
      />,
    );

    expect(screen.getByText("Requer atenção")).toBeInTheDocument();
  });

  it("renders a skeleton state when loading", () => {
    const { container } = render(<KpiCard label="Total detectados" value="14" icon={Package} loading />);

    expect(container.querySelector("[aria-hidden='true']")).toBeInTheDocument();
    expect(screen.queryByText("Total detectados")).not.toBeInTheDocument();
  });

  it("marks the value with aria-live=polite", () => {
    render(<KpiCard label="Total detectados" value="14" icon={Package} />);
    const value = screen.getByText("14");
    expect(value.getAttribute("aria-live")).toBe("polite");
  });
});
