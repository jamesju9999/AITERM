import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ProjectNotes } from "./ProjectNotes";

vi.mock("../../ipc/tasks", () => ({
  listProjectNotes: vi.fn(),
  addProjectNote: vi.fn(),
  deleteProjectNote: vi.fn(),
}));
import { addProjectNote, deleteProjectNote, listProjectNotes } from "../../ipc/tasks";

const note = (id: number, text: string, author = "工程師1") => ({ id, author, text, created_at: "" });
const renderNotes = () =>
  render(
    <LocaleProvider>
      <ProjectNotes projectId="p1" />
    </LocaleProvider>,
  );

describe("專案記事本", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(listProjectNotes).mockResolvedValue([]);
    vi.mocked(deleteProjectNote).mockResolvedValue(undefined);
  });

  it("列出各張卡留下的筆記與署名", async () => {
    vi.mocked(listProjectNotes).mockResolvedValue([note(1, "計時一律用 performance.now()"), note(2, "別動 legacy/", "")]);
    renderNotes();
    expect(await screen.findByText("計時一律用 performance.now()")).toBeInTheDocument();
    expect(screen.getByText("工程師1")).toBeInTheDocument();
    expect(screen.getByText("別動 legacy/")).toBeInTheDocument();
  });

  it("沒有筆記時說明這是什麼", async () => {
    renderNotes();
    expect(await screen.findByTestId("notes-empty")).toBeInTheDocument();
  });

  it("輸入一句按新增就存起來並出現在清單；空白不能送", async () => {
    vi.mocked(addProjectNote).mockResolvedValue(note(5, "用 pnpm", ""));
    renderNotes();
    await screen.findByTestId("notes-empty");
    expect(screen.getByTestId("note-add")).toBeDisabled();
    fireEvent.change(screen.getByTestId("note-input"), { target: { value: "  用 pnpm  " } });
    fireEvent.click(screen.getByTestId("note-add"));
    await waitFor(() => expect(addProjectNote).toHaveBeenCalledWith("p1", "用 pnpm"));
    expect(await screen.findByText("用 pnpm")).toBeInTheDocument();
    expect((screen.getByTestId("note-input") as HTMLInputElement).value).toBe("");
  });

  it("新增失敗時顯示原因並保留輸入", async () => {
    vi.mocked(addProjectNote).mockRejectedValue("筆記太長（上限 500 字），請濃縮成一兩句");
    renderNotes();
    await screen.findByTestId("notes-empty");
    fireEvent.change(screen.getByTestId("note-input"), { target: { value: "很長" } });
    fireEvent.click(screen.getByTestId("note-add"));
    expect(await screen.findByText(/筆記太長/)).toBeInTheDocument();
    expect((screen.getByTestId("note-input") as HTMLInputElement).value).toBe("很長");
  });

  it("可以刪掉一則", async () => {
    vi.mocked(listProjectNotes).mockResolvedValue([note(1, "A"), note(2, "B")]);
    renderNotes();
    await screen.findByText("A");
    fireEvent.click(screen.getByTestId("note-delete-1"));
    await waitFor(() => expect(deleteProjectNote).toHaveBeenCalledWith("p1", 1));
    await waitFor(() => expect(screen.queryByText("A")).not.toBeInTheDocument());
    expect(screen.getByText("B")).toBeInTheDocument();
  });

  it("載入失敗時顯示原因而不是整個壞掉", async () => {
    vi.mocked(listProjectNotes).mockRejectedValue("db locked");
    renderNotes();
    expect(await screen.findByText(/db locked/)).toBeInTheDocument();
  });
});
