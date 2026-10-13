import type { ProductEngineWire, ProductRuntimeMethod } from "./frame-protocol.ts";

export interface AudioPcmWindow {
  digest: string;
  startSample: number;
  left: Float32Array;
  right: Float32Array;
}

/** A message is atomic on the authoritative worker. Preview/export cannot retire one another's
 * source PCM between registration and mixing; even an unsuccessful block releases every plane. */
export function executeRuntimeCall(engine: ProductEngineWire, method: ProductRuntimeMethod, args: unknown[]): unknown {
  if (method === "render_scene3d_request" || method === "render_scene3d_request_deferred" || method === "pick_scene3d_request") {
    const calls = engine as unknown as Record<string, (...args: unknown[]) => unknown>;
    try {
      if (method === "pick_scene3d_request") {
        const [content, topology, request, x, y] = args;
        calls.render_scene3d_request!(content, topology, request);
        return calls.scene3d_pick_json!(content, x, y);
      }
      return calls[method]!(...args);
    } finally { engine.clear_scene3d_frames(); }
  }
  if (method === "mix_audio_windows") {
    const [renderId, start, end, windows] = args as [string, bigint, bigint, AudioPcmWindow[]];
    engine.clear_audio_pcm(renderId);
    try {
      for (const window of windows) {
        if (!Number.isSafeInteger(window.startSample) || window.startSample < 0) throw new Error("invalid audio window start");
        engine.register_audio_pcm_window(renderId, window.digest, BigInt(window.startSample), window.left, window.right);
      }
      return engine.mix_audio_pcm(renderId, start, end);
    } finally { engine.clear_audio_pcm(renderId); }
  }
  const operation = (engine as unknown as Record<string, (...args: unknown[]) => unknown>)[method];
  if (typeof operation !== "function") throw new Error(`Product runtime method ${method} is unavailable`);
  const result = operation.apply(engine, args);
  // wasm-bindgen copies mutable slices back into these worker-local arrays. Return that array;
  // mutating a structured clone cannot change the caller's CanvasKit memory.
  return method.startsWith("transform_srgb_preview_") ? args[0] : result;
}
