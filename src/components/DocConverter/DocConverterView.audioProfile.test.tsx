import { describe, it, expect } from "vitest";
import { needsAudioProfile } from "./DocConverterView";

describe("needsAudioProfile", () => {
  it("is true for audio files", () => {
    for (const name of ["g.mp3", "h.WAV", "i.m4a", "j.flac"]) {
      expect(needsAudioProfile(name)).toBe(true);
    }
  });

  it("is false for image files — converter.py handles images itself via its own vision-API path (falling back to Pillow, which already ships in doc_core), so no second profile is ever needed for them", () => {
    for (const name of ["a.png", "b.JPG", "c.jpeg", "d.gif", "e.bmp", "f.webp"]) {
      expect(needsAudioProfile(name)).toBe(false);
    }
  });

  it("is false for the document formats the core profile covers", () => {
    for (const name of ["a.pdf", "b.docx", "c.pptx", "d.xlsx", "e.txt"]) {
      expect(needsAudioProfile(name)).toBe(false);
    }
  });

  it("is false when there is no extension", () => {
    expect(needsAudioProfile("README")).toBe(false);
  });

  it("handles a path, not just a bare file name", () => {
    // The picker hands back a full path.
    expect(needsAudioProfile("/Users/me/Music/voice.MP3")).toBe(true);
    expect(needsAudioProfile("C:\\Users\\me\\Documents\\report.pdf")).toBe(false);
    expect(needsAudioProfile("/Users/me/Pictures/holiday.PNG")).toBe(false);
  });

  it("does not mistake a dot in a directory name for a file extension", () => {
    // A directory named "v1.2" with no actual file extension should not
    // be misread as an extension "2" (or worse).
    expect(needsAudioProfile("/Users/me/v1.2/report")).toBe(false);
  });
});
