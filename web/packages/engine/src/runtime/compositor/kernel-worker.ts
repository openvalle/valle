import { PRODUCT_FRAME_WORKER_PROTOCOL_VERSION as version, type ProductEngineWire, type ProductWasmModule } from "./frame-protocol.ts";

export interface Scene3dKernelInput { content: string; topology: string; canonical: string; width: number; height: number; }
export interface MotionKernelInput { kind: "motion"; definitionId: string; definitionJson: string; inputJson: string; compact: boolean; width: number; height: number; }
export type KernelInput = Scene3dKernelInput | MotionKernelInput;
export interface KernelMessage { version: number; id: number; wasmModuleUrl: string; wasmUrl: string; input: KernelInput; }
/** `residentBytes` is the worker's WASM heap, which keeps its high-water mark until the worker exits. */
export interface KernelResult { version: number; id: number; pixels?: Float32Array; motion?: Uint8Array; workMs?: number; residentBytes?: number; error?: string; }

/** Independent scene/geometry memoization; never opens a Timeline or compiles a fixed package. */
export function installKernelWorker(): void {
  const scope = globalThis as unknown as { onmessage: (event: MessageEvent<KernelMessage>) => void; postMessage(value: KernelResult, transfer?: Transferable[]): void };
  let engine: ProductEngineWire | undefined, memory: WebAssembly.Memory | undefined, identity = "", chain = Promise.resolve();
  scope.onmessage = event => { chain = chain.then(async () => {
    const m = event.data;
    try {
      if (m.version !== version || !Number.isSafeInteger(m.id)) throw new Error("invalid kernel message");
      const motion = "kind" in m.input && m.input.kind === "motion";
      if (motion) {
        const input = m.input as MotionKernelInput;
        if (!/^sha256:[0-9a-f]{64}$/.test(input.definitionId) || typeof input.definitionJson !== "string"
          || typeof input.inputJson !== "string" || typeof input.compact !== "boolean") throw new Error("invalid Motion kernel message");
      } else {
        const input = m.input as Scene3dKernelInput;
        if (typeof input.canonical !== "string" || !/^sha256:[0-9a-f]{64}$/.test(input.content)
          || !/^sha256:[0-9a-f]{64}$/.test(input.topology)) throw new Error("invalid Scene3D kernel message");
        const descriptor = JSON.parse(input.canonical);
        if (descriptor.width !== input.width || descriptor.height !== input.height) throw new Error("kernel extent does not match the admitted request");
      }
      const key = JSON.stringify([m.wasmModuleUrl, m.wasmUrl]);
      if (engine && key !== identity) throw new Error("kernel runtime identity changed");
      if (!engine) {
        const wasm = await import(m.wasmModuleUrl) as ProductWasmModule;
        memory = (await wasm.default(m.wasmUrl)).memory; engine = new wasm.ProductEngine(); identity = key;
      }
      const started = performance.now();
      if (motion) {
        const input = m.input as MotionKernelInput;
        const bytes = engine.run_motion_kernel!(input.definitionId, input.definitionJson, input.inputJson, input.compact);
        scope.postMessage({ version, id: m.id, motion: bytes, workMs: performance.now() - started, residentBytes: memory!.buffer.byteLength }, [bytes.buffer as ArrayBuffer]);
      } else {
        const input = m.input as Scene3dKernelInput;
        const pixels = engine.scene3d_gpu_lighting_input!(input.content, input.topology, input.canonical);
        scope.postMessage({ version, id: m.id, pixels, workMs: performance.now() - started, residentBytes: memory!.buffer.byteLength }, [pixels.buffer as ArrayBuffer]);
      }
    } catch (error) { scope.postMessage({ version, id: m.id, error: String(error) }); }
  }); };
}
