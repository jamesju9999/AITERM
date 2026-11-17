import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import { RepoStatus } from "./RepoStatus";

const mount = (nested_in: string | null, dirty = false) =>
  render(
    <LocaleProvider>
      <RepoStatus repo={{ status: "ok", branch: "main", nested_in, dirty }} busy={false} onFix={() => {}} />
    </LocaleProvider>,
  );

describe("RepoStatus nested repo warning", () => {
  it("names the enclosing repo when the folder is only a subfolder of it", () => {
    mount("/Users/me/GitCodeBase");
    expect(screen.getByTestId("repo-nested").textContent).toContain("/Users/me/GitCodeBase");
  });

  it("shows nothing extra for a normal repo", () => {
    mount(null);
    expect(screen.queryByTestId("repo-nested")).toBeNull();
  });
});

describe("RepoStatus uncommitted changes warning", () => {
  it("warns when the repo has uncommitted changes", () => {
    mount(null, true);
    expect(screen.getByTestId("repo-dirty")).toBeTruthy();
  });

  it("stays quiet for a clean repo", () => {
    mount(null, false);
    expect(screen.queryByTestId("repo-dirty")).toBeNull();
  });
});
