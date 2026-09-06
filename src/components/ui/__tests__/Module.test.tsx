import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { Module } from "../Module";

describe("Module", () => {
  it("renders title, subtitle, status slot, children and footer", () => {
    render(
      <Module title="Google Drive" subtitle="Service Account" status={<span>Conectado</span>} footer={<span>Testar Conexão</span>}>
        <p>corpo</p>
      </Module>,
    );

    expect(screen.getByRole("heading", { name: "Google Drive" })).toBeInTheDocument();
    expect(screen.getByText("Service Account")).toBeInTheDocument();
    expect(screen.getByText("Conectado")).toBeInTheDocument();
    expect(screen.getByText("corpo")).toBeInTheDocument();
    expect(screen.getByText("Testar Conexão")).toBeInTheDocument();
  });
});
