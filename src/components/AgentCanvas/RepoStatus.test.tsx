import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import { RepoStatus } from "./RepoStatus";

const mount = (nested_in: string | null) =>
  render(
    <LocaleProvider>
      <RepoStatus repo={{ status: "ok", branch: "main", nested_in }} busy={false} onFix={() => {}} />
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
