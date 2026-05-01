import { describe, it, expect } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { vi } from "vitest";
import { VcsConnectionsPage } from "./VcsConnectionsPage";

vi.mock("../../ipc/vcs", () => ({
  vcsListConnections: vi.fn().mockResolvedValue([]),
  vcsAddConnection: vi.fn().mockResolvedValue("new-id"),
  vcsRemoveConnection: vi.fn().mockResolvedValue(undefined),
  vcsTestConnection: vi.fn().mockResolvedValue("Connection successful"),
  vcsUpdateConnection: vi.fn().mockResolvedValue(undefined),
}));

describe("VcsConnectionsPage", () => {
  it("renders without crashing", async () => {
    render(<VcsConnectionsPage />);
    await waitFor(() => expect(screen.getByText("VCS 連線")).toBeInTheDocument());
  });

  it("shows no connections empty state when list is empty", async () => {
    render(<VcsConnectionsPage />);
    await waitFor(() => expect(screen.getByText("尚無 VCS 連線。點擊「+ 新增連線」開始。")).toBeInTheDocument());
  });
});
