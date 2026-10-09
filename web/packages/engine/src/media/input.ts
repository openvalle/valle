import { ALL_FORMATS, BlobSource, BufferSource, Input } from "mediabunny";

export interface BrowserMediaInput {
  buffer?: ArrayBuffer | Uint8Array;
  blob?: Blob;
}

/** Share one media reader; BlobSource keeps large encoded files out of the JS heap. */
export function createMediaInput({ buffer, blob }: BrowserMediaInput): Input {
  if (!blob && !buffer) throw new Error("A media source requires a Blob or encoded buffer.");
  return new Input({ formats: ALL_FORMATS, source: blob ? new BlobSource(blob) : new BufferSource(buffer!) });
}
