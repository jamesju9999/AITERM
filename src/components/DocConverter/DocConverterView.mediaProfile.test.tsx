import { describe, it, expect } from "vitest";
import { needsMediaProfile } from "./DocConverterView";

describe("needsMediaProfile", () => {
  it("is true for image and audio files", () => {
    for (const name of ["a.png", "b.JPG", "c.jpeg", "d.gif", "e.bmp", "f.webp", "g.mp3", "h.wav", "i.m4a", "j.flac"]) {
      expect(needsMediaProfile(name)).toBe(true);
    }
  });

  it("is false for the document formats the core profile covers", () => {
    for (const name of ["a.pdf", "b.docx", "c.pptx", "d.xlsx", "e.txt"]) {
      expect(needsMediaProfile(name)).toBe(false);
    }
  });

  it("is false when there is no extension", () => {
    expect(needsMediaProfile("README")).toBe(false);
  });

  it("handles a path, not just a bare file name", () => {
    // The picker hands back a full path.
    expect(needsMediaProfile("/Users/me/Pictures/holiday.PNG")).toBe(true);
    expect(needsMediaProfile("C:\\Users\\me\\Documents\\report.pdf")).toBe(false);
  });

  it("does not mistake a dot in a directory name for a file extension", () => {
    // A directory named "v1.2" with no actual file extension should not
    // be misread as an extension "2" (or worse).
    expect(needsMediaProfile("/Users/me/v1.2/report")).toBe(false);
  });
});
