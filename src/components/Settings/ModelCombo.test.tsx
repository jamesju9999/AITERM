import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ModelCombo } from "./ModelCombo";

describe("ModelCombo", () => {
  it("lists every option and picks one by clicking it", () => {
    const onChange = vi.fn();
    render(<ModelCombo value="a" options={["a", "b", "c-last"]} onChange={onChange} />);
    expect(screen.queryByRole("listbox")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "models" }));
    expect(screen.getAllByRole("option")).toHaveLength(3);
    fireEvent.click(screen.getByText("c-last"));
    expect(onChange).toHaveBeenCalledWith("c-last");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("lets the user type a model id that is not in the list", () => {
    const onChange = vi.fn();
    render(<ModelCombo value="a" options={["a"]} onChange={onChange} />);
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "claude-custom" } });
    expect(onChange).toHaveBeenCalledWith("claude-custom");
  });

  it("closes when clicking outside", () => {
    render(<div><ModelCombo value="a" options={["a", "b"]} onChange={() => {}} /><p>outside</p></div>);
    fireEvent.click(screen.getByRole("button", { name: "models" }));
    expect(screen.getByRole("listbox")).toBeTruthy();
    fireEvent.mouseDown(screen.getByText("outside"));
    expect(screen.queryByRole("listbox")).toBeNull();
  });
});
