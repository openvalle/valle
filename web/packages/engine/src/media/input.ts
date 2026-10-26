import { ALL_FORMATS, BlobSource, BufferSource, Input, UrlSource } from "mediabunny";

export interface BrowserMediaInput {
  buffer?: ArrayBuffer | Uint8Array;
  blob?: Blob;
  url?: string;
}

/** Share one media reader; URLs use bounded range reads rather than a whole-file Blob. */
export function createMediaInput({ buffer, blob, url }: BrowserMediaInput): Input {
  if (!url && !blob && !buffer)
    throw new Error("A media source requires a URL, Blob or encoded buffer.");
  return new Input({
    formats: ALL_FORMATS,
    source: url
      ? new UrlSource(url, { maxCacheSize: 16 * 1024 ** 2, getRetryDelay: () => null })
      : blob
        ? new BlobSource(blob)
        : new BufferSource(buffer!),
  });
}
