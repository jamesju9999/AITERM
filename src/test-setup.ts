import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

// jsdom does not implement scrollIntoView
window.HTMLElement.prototype.scrollIntoView = () => {};

afterEach(() => {
  cleanup();
});
