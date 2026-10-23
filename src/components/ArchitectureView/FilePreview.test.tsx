import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { LocaleProvider } from "../../contexts/LocaleContext";

const archReadFile = vi.fn();
const archOpenPath = vi.fn();
vi.mock("../../ipc/architecture", () => ({
  archReadFile: (...a: unknown[]) => archReadFile(...a),
  archOpenPath: (...a: unknown[]) => archOpenPath(...a),
}));

import { FilePreview } from "./FilePreview";

function setup(props: Partial<React.ComponentProps<typeof FilePreview>> = {}) {
  const onClose = vi.fn();
  render(
    <LocaleProvider>
      <FilePreview root="/proj" path="src/a.ts" onClose={onClose} {...props} />
    </LocaleProvider>,
  );
  return onClose;
}

beforeEach(() => {
  vi.clearAllMocks();
  archOpenPath.mockResolvedValue(undefined);
  archReadFile.mockResolvedValue({ content: "alpha\nbeta\ngamma\n", truncated: false, total_bytes: 17 });
});

describe("FilePreview", () => {
  it("讀取指定檔案並顯示內容與行號", async () => {
    setup();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(await screen.findByText("beta")).toBeInTheDocument();
    expect(archReadFile).toHaveBeenCalledWith("/proj", "src/a.ts");
    const rows = document.querySelectorAll("[data-line]");
    expect(rows).toHaveLength(3); // 結尾換行不產生第 4 個空行
    expect(rows[1].getAttribute("data-line")).toBe("2");
    expect(rows[1].textContent).toContain("beta");
    expect(screen.getByTitle("src/a.ts")).toBeInTheDocument();
  });

  it("line 屬性會標出該行，其他行不標", async () => {
    setup({ line: 2 });
    await screen.findByText("beta");
    const hit = document.querySelectorAll('[aria-current="true"]');
    expect(hit).toHaveLength(1);
    expect(hit[0].getAttribute("data-line")).toBe("2");
  });

  it("Esc 與關閉鈕都會關閉", async () => {
    const onClose = setup();
    await screen.findByText("beta");
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: /關閉預覽|Close preview/ }));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("複製路徑寫入專案根接上相對路徑的絕對路徑（根尾端斜線不重複）", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    setup({ root: "/proj/" });
    await screen.findByText("beta");
    fireEvent.click(screen.getByRole("button", { name: /複製路徑|Copy path/ }));
    expect(writeText).toHaveBeenCalledWith("/proj/src/a.ts");
  });

  it("剪貼簿不可用時複製鈕不會丟錯", async () => {
    Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
    setup();
    await screen.findByText("beta");
    expect(() => fireEvent.click(screen.getByRole("button", { name: /複製路徑|Copy path/ }))).not.toThrow();
  });

  it("用預設程式開啟呼叫 archOpenPath(root, path)", async () => {
    setup();
    await screen.findByText("beta");
    fireEvent.click(screen.getByRole("button", { name: /用預設程式開啟|Open with default app/ }));
    expect(archOpenPath).toHaveBeenCalledWith("/proj", "src/a.ts");
  });

  it("讀取失敗時顯示後端訊息", async () => {
    archReadFile.mockRejectedValue("這是二進位檔，無法預覽");
    setup();
    expect(await screen.findByRole("alert")).toHaveTextContent("這是二進位檔，無法預覽");
  });

  it("截斷時顯示提示", async () => {
    archReadFile.mockResolvedValue({ content: "x".repeat(2048), truncated: true, total_bytes: 9999999 });
    setup();
    expect(await screen.findByText(/已截斷，只顯示前 2 KB|Truncated, showing the first 2 KB/)).toBeInTheDocument();
  });

  it("沒截斷時不顯示截斷提示", async () => {
    setup();
    await screen.findByText("beta");
    expect(screen.queryByText(/已截斷|Truncated/)).not.toBeInTheDocument();
  });

  it("最多渲染 3000 行並附說明", async () => {
    archReadFile.mockResolvedValue({ content: Array.from({ length: 3500 }, (_, i) => `l${i + 1}`).join("\n"), truncated: false, total_bytes: 1 });
    setup();
    await waitFor(() => expect(document.querySelectorAll("[data-line]").length).toBe(3000));
    expect(screen.getByText(/只顯示前 3000 行|first 3000 lines/)).toBeInTheDocument();
  });

  it("換檔時重新讀取新檔案", async () => {
    const { rerender } = render(
      <LocaleProvider><FilePreview root="/proj" path="a.ts" onClose={vi.fn()} /></LocaleProvider>,
    );
    await screen.findByText("beta");
    archReadFile.mockResolvedValue({ content: "other", truncated: false, total_bytes: 5 });
    rerender(<LocaleProvider><FilePreview root="/proj" path="b.ts" onClose={vi.fn()} /></LocaleProvider>);
    expect(await screen.findByText("other")).toBeInTheDocument();
    expect(archReadFile).toHaveBeenLastCalledWith("/proj", "b.ts");
  });
});

describe("語法上色", () => {
  it("認得的副檔名會把關鍵字包成 hljs span，文字內容不變", async () => {
    archReadFile.mockResolvedValue({ content: "public class A {}\n", truncated: false, total_bytes: 18 });
    const { container } = render(<LocaleProvider><FilePreview root="/r" path="A.java" onClose={() => {}} /></LocaleProvider>);
    await waitFor(() => expect(container.querySelector(".hljs-keyword")?.textContent).toBe("public"));
    expect(container.querySelector('[data-line="1"]')?.textContent).toContain("public class A {}");
  });
  it("不認得的檔案維持純文字、沒有 hljs span", async () => {
    archReadFile.mockResolvedValue({ content: "public class A {}\n", truncated: false, total_bytes: 18 });
    const { container } = render(<LocaleProvider><FilePreview root="/r" path="NOTES" onClose={() => {}} /></LocaleProvider>);
    await waitFor(() => expect(container.querySelector('[data-line="1"]')).not.toBeNull());
    expect(container.querySelector(".hljs-keyword")).toBeNull();
  });
});
