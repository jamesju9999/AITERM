import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const createProject = vi.fn();
const openDialog = vi.fn();

vi.mock("../../ipc/projects", () => ({
  createProject: (...a: unknown[]) => createProject(...a),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (...a: unknown[]) => openDialog(...a) }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ProjectCreateDialog } from "./ProjectCreateDialog";

const mount = (onCreated = vi.fn()) =>
  render(
    <LocaleProvider>
      <ProjectCreateDialog onClose={vi.fn()} onCreated={onCreated} />
    </LocaleProvider>,
  );

describe("ProjectCreateDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    createProject.mockResolvedValue("new-id");
  });

  it("名稱為空時無法送出", async () => {
    mount();
    expect(screen.getByTestId("project-create-submit")).toBeDisabled();
  });

  it("沒有選建立位置時無法送出", async () => {
    mount();
    await userEvent.type(screen.getByTestId("project-create-name"), "makemoney");
    expect(screen.getByTestId("project-create-submit")).toBeDisabled();
  });

  it("名稱與位置都齊了才送出，並回報新 id", async () => {
    openDialog.mockResolvedValue("/Users/me/Projects");
    const onCreated = vi.fn();
    mount(onCreated);

    await userEvent.type(screen.getByTestId("project-create-name"), "makemoney");
    await userEvent.click(screen.getByTestId("project-create-browse"));
    await waitFor(() =>
      expect(screen.getByTestId("project-create-submit")).not.toBeDisabled(),
    );
    await userEvent.click(screen.getByTestId("project-create-submit"));

    await waitFor(() =>
      expect(createProject).toHaveBeenCalledWith({
        parentDir: "/Users/me/Projects",
        name: "makemoney",
        description: "",
      }),
    );
    expect(onCreated).toHaveBeenCalledWith("new-id");
  });

  it("記住上次的建立位置", async () => {
    localStorage.setItem("aiterm_last_project_parent", "/remembered");
    mount();
    await userEvent.type(screen.getByTestId("project-create-name"), "x");
    await waitFor(() =>
      expect(screen.getByTestId("project-create-submit")).not.toBeDisabled(),
    );
  });
});
