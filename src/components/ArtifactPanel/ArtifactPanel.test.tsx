import { useEffect } from "react";
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const save = vi.fn();
const writeTextFile = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: (...a: unknown[]) => save(...a) }));
vi.mock("../../ipc/fs", () => ({ writeTextFile: (...a: unknown[]) => writeTextFile(...a) }));
import {
  ArtifactPanelProvider,
  useArtifactPanel,
  type Artifact,
} from "../../contexts/ArtifactPanelContext";
import { ArtifactPanel } from "./ArtifactPanel";

function ShowOnMount({ artifact }: { artifact: Artifact }) {
  const { showArtifact } = useArtifactPanel();
  useEffect(() => {
    showArtifact(artifact);
  }, [artifact, showArtifact]);
  return null;
}

describe("ArtifactPanel", () => {
  beforeEach(() => {
    save.mockReset();
    writeTextFile.mockReset();
  });

  it("renders nothing when there is no active artifact", () => {
    const { container } = render(
      <ArtifactPanelProvider>
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    expect(container.querySelector(".aiterm-artifact-panel")).toBeNull();
  });

  it("renders an html artifact inside an iframe with the artifact's title", () => {
    render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={{ id: "1", kind: "html", title: "Brief", content: "<p>hi</p>" }} />
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    expect(screen.getByText("Brief")).toBeInTheDocument();
    expect(document.querySelector("iframe")).not.toBeNull();
  });

  it("shows an error message when chart content is not valid JSON", () => {
    render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={{ id: "2", kind: "chart", title: "Bad", content: "not json" }} />
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    expect(screen.getByText("圖表資料格式錯誤，無法解析。")).toBeInTheDocument();
  });

  it("offers a download button for an html artifact and writes what was rendered", async () => {
    save.mockResolvedValue("/tmp/out.html");
    render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={{ id: "1", kind: "html", title: "Brief", content: "<p>hi</p>" }} />
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    fireEvent.click(screen.getByTitle("下載 HTML 文件"));
    await waitFor(() => expect(writeTextFile).toHaveBeenCalledWith("/tmp/out.html", "<p>hi</p>"));
    // 預設檔名該用文件標題，而不是一個沒有意義的 untitled。
    expect(save.mock.calls[0][0]).toMatchObject({ defaultPath: "Brief.html" });
  });

  it("writes nothing when the save dialog is cancelled", async () => {
    save.mockResolvedValue(null);
    render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={{ id: "1", kind: "html", title: "Brief", content: "<p>hi</p>" }} />
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    fireEvent.click(screen.getByTitle("下載 HTML 文件"));
    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(writeTextFile).not.toHaveBeenCalled();
  });

  // 標題直接來自模型寫的 <title>，可能含有路徑分隔字元之類的東西。
  it("sanitises a title that would be an illegal filename", async () => {
    save.mockResolvedValue("/tmp/out.html");
    render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={{ id: "1", kind: "html", title: "a/b:c*d", content: "<p>hi</p>" }} />
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    fireEvent.click(screen.getByTitle("下載 HTML 文件"));
    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(save.mock.calls[0][0].defaultPath).toBe("a_b_c_d.html");
  });

  // 圖表是結構化資料不是文件，這個按鈕對它沒有意義（要匯出圖片是另一件事）。
  it("does not offer the download button for a chart artifact", () => {
    render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={{ id: "2", kind: "chart", title: "Bad", content: "not json" }} />
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    expect(screen.queryByTitle("下載 HTML 文件")).toBeNull();
  });

  // JSON.parse 只保證「是合法 JSON」，不保證「是 ChartSpec」。模型吐出 {} 或
  // 少了 series 的物件時，ArtifactChart 會在 spec.series.length 上直接炸掉，把
  // 整個面板帶走——這是模型輸出就能觸發的崩潰，必須擋在渲染之前。
  it("shows the error message instead of crashing when chart JSON has the wrong shape", () => {
    for (const bad of ["{}", '{"type":"bar"}', '{"type":"bar","data":[],"xKey":"x"}']) {
      const { unmount } = render(
        <ArtifactPanelProvider>
          <ShowOnMount artifact={{ id: "3", kind: "chart", title: "Bad", content: bad }} />
          <ArtifactPanel />
        </ArtifactPanelProvider>,
      );
      expect(screen.getByText("圖表資料格式錯誤，無法解析。")).toBeInTheDocument();
      unmount();
    }
  });

  it("clicking close clears the active artifact", () => {
    render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={{ id: "1", kind: "html", title: "Brief", content: "<p>hi</p>" }} />
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    fireEvent.click(screen.getByTitle("關閉文件面板"));
    expect(screen.queryByText("Brief")).not.toBeInTheDocument();
  });
});
