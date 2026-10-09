import { expect, test } from "bun:test";
import { createExportFileSink } from "./export-file-sink.ts";

function file() {
  const result = { committed: false, aborted: false, reason: undefined as unknown, writes: 0 };
  const stream = new WritableStream({
    write() { result.writes++; },
    close() { result.committed = true; },
    abort(reason) { result.aborted = true; result.reason = reason; },
  }) as FileSystemWritableFileStream;
  return { stream, result };
}

test("closing the muxer target on cancellation never commits an incomplete MP4", async () => {
  const { stream, result } = file();
  const sink = createExportFileSink(stream);
  const muxer = sink.stream.getWriter();
  await muxer.write({ type: "write", position: 0, data: new Uint8Array([1, 2, 3]) });
  await muxer.close();
  expect(result.writes).toBe(1);
  expect(result.committed).toBe(false);
  const error = new DOMException("Export cancelled", "AbortError");
  await sink.abort(error);
  expect(result.aborted).toBe(true);
  expect(result.reason).toBe(error);
  expect(result.committed).toBe(false);
  expect(stream.locked).toBe(false);
});

test("a complete MP4 commits the file only after finalization succeeds", async () => {
  const { stream, result } = file();
  const sink = createExportFileSink(stream);
  const muxer = sink.stream.getWriter();
  await muxer.write({ type: "write", position: 0, data: new Uint8Array([1, 2, 3]) });
  await muxer.close();
  expect(result.committed).toBe(false);
  await sink.commit();
  expect(result.committed).toBe(true);
  expect(result.aborted).toBe(false);
  expect(stream.locked).toBe(false);
});
