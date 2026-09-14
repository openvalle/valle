/** A TTF/OTF URL or already loaded bytes. Order determines fallback priority. */
export type MotionFontSource = string | Uint8Array;

/** One player-local lazy load; no global registry or fixed font/CDN dependency. */
export function createMotionFontLoader(sources: readonly MotionFontSource[], baseUrl: string): () => Promise<string> {
  const inputs = sources.map((source) => typeof source === "string" ? source : source.slice());
  let pending: Promise<string> | undefined;
  return () => {
    if (pending) return pending;
    const urls = new Map<string, Promise<Uint8Array>>();
    pending = Promise.all(inputs.map(async (source) => {
      let bytes: Uint8Array;
      if (typeof source === "string") {
        const url = new URL(source, baseUrl).href;
        let loading = urls.get(url);
        if (!loading) {
          loading = fetch(url).then(async (response) => {
            if (!response.ok) throw new Error(`font request failed (${response.status}): ${url}`);
            return new Uint8Array(await response.arrayBuffer());
          });
          urls.set(url, loading);
        }
        bytes = await loading;
      } else {
        bytes = source;
      }
      if (bytes.byteLength === 0) throw new Error("font bytes must not be empty");
      let binary = "";
      for (let at = 0; at < bytes.length; at += 0x8000) {
        binary += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
      }
      return { bytesBase64: btoa(binary) };
    })).then((fonts) => JSON.stringify(fonts));
    void pending.catch(() => { pending = undefined; });
    return pending;
  };
}
