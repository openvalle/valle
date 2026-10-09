import type { StreamTargetChunk } from "mediabunny";

/** Keep the file's transactional write open until encoding and muxing have both succeeded. */
export function createExportFileSink(writable: FileSystemWritableFileStream) {
  const writer = writable.getWriter();
  return {
    // A muxer cancellation closes its target stream. That must not commit an incomplete file.
    stream: new WritableStream<StreamTargetChunk>({ write: (chunk) => writer.write(chunk) }),
    async commit() {
      try { await writer.close(); } finally { writer.releaseLock(); }
    },
    async abort(reason: unknown) {
      try { await writer.abort(reason); } finally { writer.releaseLock(); }
    },
  };
}
