import { describe, expect, it } from "vitest";

import { GITHUB_RELEASES_URL, releaseTagUrl } from "./repo";

describe("releaseTagUrl", () => {
  it("points at the specific tag, not the latest release", () => {
    // The modal is telling the user about *this* version. Sending them to
    // /releases/latest would show a different release the moment a newer one
    // ships, which is exactly when this link matters most.
    expect(releaseTagUrl("1.2.5")).toBe(
      "https://github.com/jamesju9999/AITERM/releases/tag/v1.2.5",
    );
    expect(releaseTagUrl("1.2.5")).not.toBe(GITHUB_RELEASES_URL);
  });

  it("does not double the v prefix", () => {
    expect(releaseTagUrl("1.2.5")).toContain("/tag/v1.2.5");
    expect(releaseTagUrl("1.2.5")).not.toContain("/tag/vv");
  });
});
