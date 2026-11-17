import { fireEvent, render, screen } from "@testing-library/react";
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

function mount(list: Recipe[], onDelete = vi.fn()) {
  render(
    <LocaleProvider>
      <RecipeList recipes={list} onDelete={onDelete} />
    </LocaleProvider>,
  );
  return onDelete;
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
    const onDelete = mount([recipe()]);
    expect(screen.queryByTestId("recipe-steps")).toBeNull();
    fireEvent.click(screen.getByText(t.sys_recipe_show_steps));
    expect(screen.getByTestId("recipe-steps").textContent).toContain("寫：送出表單");
    fireEvent.click(screen.getByText(t.sys_delete));
    expect(onDelete).toHaveBeenCalledWith(expect.objectContaining({ id: 1, name: "送單" }));
  });
});
