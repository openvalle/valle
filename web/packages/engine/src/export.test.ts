import { expect, test } from "bun:test";
import { exportBrowserVideo } from "./export.ts";
import type { BrowserValleWebPlayerOptions } from "./runtime/product-controller.ts";

test("an already cancelled export aborts its destination before loading resources", async () => {
  let aborted: unknown;
  let committed = false;
  const writable = new WritableStream({
    close() { committed = true; },
    abort(reason) { aborted = reason; },
  }) as FileSystemWritableFileStream;
  const cancellation = new DOMException("Cancelled before export", "AbortError");
  const input = new Proxy({} as BrowserValleWebPlayerOptions, {
    get() { throw new Error("cancelled export tried to bootstrap a renderer"); },
    ownKeys() { throw new Error("cancelled export tried to read its package"); },
  });
  await expect(exportBrowserVideo(input, { writable, signal: AbortSignal.abort(cancellation) })).rejects.toBe(cancellation);
  expect(aborted).toBe(cancellation);
  expect(committed).toBe(false);
  expect(writable.locked).toBe(false);
});
