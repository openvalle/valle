/** Preserve the Host's manifest and source bytes for the Wasm shader admission path. */
export function frozenShaderBytes(manifest: Uint8Array, source: Uint8Array): Uint8Array {
  const decoder = new TextDecoder("utf-8", { fatal: true });
  return new TextEncoder().encode(JSON.stringify({
    manifest: JSON.parse(decoder.decode(manifest)) as unknown,
    source: decoder.decode(source),
  }));
}
