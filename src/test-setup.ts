import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

// jsdom does not implement scrollIntoView
window.HTMLElement.prototype.scrollIntoView = () => {};

// Newer Node versions ship their own built-in `localStorage` global, which can
// shadow jsdom's implementation and (without a `--localstorage-file` path)
// leaves `localStorage.getItem` undefined. Force a plain in-memory Storage
// polyfill so tests behave the same regardless of the Node version running them.
class MemoryStorage implements Storage {
  private store = new Map<string, string>();

  get length(): number {
    return this.store.size;
  }

  clear(): void {
    this.store.clear();
  }

  getItem(key: string): string | null {
    return this.store.has(key) ? this.store.get(key)! : null;
  }

  key(index: number): string | null {
    return Array.from(this.store.keys())[index] ?? null;
  }

  removeItem(key: string): void {
    this.store.delete(key);
  }

  setItem(key: string, value: string): void {
    this.store.set(key, String(value));
  }
}

const memoryStorage = new MemoryStorage();
Object.defineProperty(globalThis, "localStorage", { value: memoryStorage, writable: true, configurable: true });
Object.defineProperty(window, "localStorage", { value: memoryStorage, writable: true, configurable: true });

afterEach(() => {
  memoryStorage.clear();
});

afterEach(() => {
  cleanup();
});
