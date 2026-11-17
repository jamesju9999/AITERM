import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { Recipe } from "../../ipc/systems";
import { translations } from "../../lib/i18n";
import { RecipeList } from "./RecipeList";

const t = translations["zh-TW"];
const recipe = (over: Partial<Recipe> = {}): Recipe => ({
  id: 1,
  system_id: "s1",
  name: "送單",
  description: "填單號並送出",
  params: [{ name: "order_no", example: "A-102" }],
  steps: [
    { tool: "sys_navigate", args: {}, intent: "讀：開表單" },
    { tool: "sys_click", args: {}, intent: "寫：送出表單" },
  ],
  ok_count: 3,
  fail_count: 1,
  last_run_ms: 5,
  ...over,
});

function mount(list: Recipe[], onDelete = vi.fn(), onRun = vi.fn(() => Promise.resolve("已完成"))) {
  render(
    <LocaleProvider>
      <RecipeList recipes={list} onDelete={onDelete} onRun={onRun} />
    </LocaleProvider>,
  );
  return { onDelete, onRun };
}

describe("RecipeList", () => {
  it("shows an empty hint", () => {
    mount([]);
    expect(screen.getByText(t.sys_recipes_empty)).toBeTruthy();
  });

  it("shows name, step count, parameters and run stats", () => {
    mount([recipe()]);
    expect(screen.getByText("送單")).toBeTruthy();
    expect(screen.getByText(t.sys_recipe_steps(2))).toBeTruthy();
    expect(screen.getByText(t.sys_recipe_stats(3, 1))).toBeTruthy();
    expect(screen.getByText(/order_no/)).toBeTruthy();
  });

  it("expands the steps with their intents, and reports delete", () => {
    const { onDelete } = mount([recipe()]);
    expect(screen.queryByTestId("recipe-steps")).toBeNull();
    fireEvent.click(screen.getByText(t.sys_recipe_show_steps));
    expect(screen.getByTestId("recipe-steps").textContent).toContain("寫：送出表單");
    fireEvent.click(screen.getByText(t.sys_delete));
    expect(onDelete).toHaveBeenCalledWith(expect.objectContaining({ id: 1, name: "送單" }));
  });

  it("run shows one input per parameter, needs them filled, and reports the replay result", async () => {
    const { onRun } = mount([recipe()]);
    fireEvent.click(screen.getByTestId("recipe-run"));
    const input = screen.getByPlaceholderText("A-102");
    const go = screen.getByText(t.sys_recipe_run_go) as HTMLButtonElement;
    expect(go.disabled).toBe(true);
    fireEvent.change(input, { target: { value: "B-7" } });
    expect(go.disabled).toBe(false);
    fireEvent.click(go);
    await waitFor(() => expect(onRun).toHaveBeenCalledWith(expect.objectContaining({ id: 1 }), { order_no: "B-7" }));
    expect((await screen.findByTestId("recipe-run-result")).textContent).toBe("已完成");
  });

  it("a recipe without parameters can run straight away, and a failed replay shows the reason", async () => {
    const { onRun } = mount([recipe({ params: [] })], vi.fn(), vi.fn(() => Promise.reject("在第 1/2 步停下")));
    fireEvent.click(screen.getByTestId("recipe-run"));
    expect(screen.getByText(t.sys_recipe_run_none)).toBeInTheDocument();
    fireEvent.click(screen.getByText(t.sys_recipe_run_go));
    await waitFor(() => expect(onRun).toHaveBeenCalledWith(expect.anything(), {}));
    expect((await screen.findByTestId("recipe-run-result")).textContent).toContain("在第 1/2 步停下");
  });
});
