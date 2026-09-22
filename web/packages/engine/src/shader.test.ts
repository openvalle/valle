import { expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { crc32, deflateSync } from "node:zlib";
import type { CanvasKit } from "canvaskit-wasm";
import { initSync, ProductEngine } from "../generated/web/valle_engine.js";
import { decodePackedAbi, RESOURCE_REQUESTS_ABI, BOUND_PROGRAM_SCHEDULES_ABI } from "./abi/packed.ts";
import { CanvasKitExecutor, type CanvasKitExternalObject } from "./executor/canvaskit/executor.ts";

// Build the local CLI and WASM before this integration test. Both backends use a
// package compiled from these author files; no generated shader/image fixtures are stored.
const root = resolve(import.meta.dir, "../../../../");
const cli = process.env.VALLE_TEST_CLI ?? join(root, "target/debug/valle");
// Run with VALLE_TEST_NATIVE_BACKEND=metal outside the sandbox to exercise the GPU.
const nativeBackend = process.env.VALLE_TEST_NATIVE_BACKEND ?? "raster";
if (nativeBackend !== "raster" && nativeBackend !== "metal") throw new Error("unsupported test backend");
const { default: CanvasKitInit } = await import("canvaskit-wasm/full") as unknown as {
  default: (options: { locateFile(file: string): string }) => Promise<CanvasKit>;
};

// Generate a tiny PNG directly so image encoders cannot premultiply away transparent RGB.
function numericPng(values: number[], bits: 8 | 16): Uint8Array {
  const concat = (...chunks: Uint8Array[]) => {
    const bytes = new Uint8Array(chunks.reduce((size, chunk) => size + chunk.length, 0));
    let offset = 0;
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
    return bytes;
  };
  const chunk = (type: string, bytes: Uint8Array) => {
    const data = concat(new TextEncoder().encode(type), bytes);
    const size = new Uint8Array(4), crc = new Uint8Array(4);
    new DataView(size.buffer).setUint32(0, bytes.length);
    new DataView(crc.buffer).setUint32(0, crc32(data));
    return concat(size, data, crc);
  };
  const header = new Uint8Array(13);
  const headerView = new DataView(header.buffer);
  headerView.setUint32(0, 2); headerView.setUint32(4, 1);
  header[8] = bits; header[9] = 6;
  const pixels = new Uint8Array(1 + values.length * bits / 8);
  const pixelView = new DataView(pixels.buffer);
  values.forEach((v, i) => bits === 16 ? pixelView.setUint16(1 + i * 2, v) : pixels[1 + i] = v);
  return concat(new Uint8Array([137,80,78,71,13,10,26,10]), chunk("IHDR", header), chunk("IDAT", deflateSync(pixels)), chunk("IEND", new Uint8Array(0)));
}

type Edge = readonly [number, number, number, number];

// Edge locations come from the authored geometry, never from differing pixels.
// Only the one-device-pixel AA band may differ in linear-light coverage.
function pixelDifference(actual: Uint8Array, expected: Uint8Array, width: number, edges: readonly Edge[] = []) {
  const linear = (byte: number) => {
    const value = byte / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  let interior = 0, edge = 0, total = 0;
  for (let p = 0; p < actual.length / 4; p++) {
    const x = p % width + 0.5, y = Math.floor(p / width) + 0.5;
    const atEdge = edges.some(([x0, y0, x1, y1]) => {
      const dx = x1 - x0, dy = y1 - y0;
      const t = Math.max(0, Math.min(1, ((x - x0) * dx + (y - y0) * dy) / (dx * dx + dy * dy)));
      return Math.hypot(x - x0 - t * dx, y - y0 - t * dy) <= 1;
    });
    const aa = actual[p * 4 + 3]! / 255, ea = expected[p * 4 + 3]! / 255;
    for (let c = 0; c < 4; c++) {
      const a = actual[p * 4 + c]!, e = expected[p * 4 + c]!;
      const difference = c === 3 ? Math.abs(aa - ea) : Math.abs(linear(a) * aa - linear(e) * ea);
      total += difference;
      if (atEdge) edge = Math.max(edge, difference);
      else interior = Math.max(interior, Math.abs(a - e));
    }
  }
  return { interior, edge, mean: total / actual.length };
}

test("frozen shaders align Native/CanvasKit sampling and arbitrary-frame results", async () => {
  const ck = await CanvasKitInit({ locateFile: () => Bun.resolveSync("canvaskit-wasm/bin/full/canvaskit.wasm", import.meta.dir) });
  initSync({ module: await readFile(new URL("../generated/web/valle_engine_bg.wasm", import.meta.url)) });
  const dir = await mkdtemp(join(tmpdir(), "valle-shader-parity-"));
  const assets = ["--asset", "effect=effect.shader.json", "--asset", "image=steps.png", "--fps", "30"];
  const spawn = (args: string[]) => Bun.spawn([cli, "--json", ...args], {
    cwd: dir, env: { ...process.env, VALLE_HOME: join(dir, "home") }, stdout: "pipe", stderr: "pipe",
  });
  const info = { width: 64, height: 64, colorType: ck.ColorType.RGBA_8888, alphaType: ck.AlphaType.Unpremul, colorSpace: ck.ColorSpace.SRGB };
  try {
    const image = ck.MakeImage({ ...info, width: 2, height: 1 }, new Uint8Array([0,0,0,255, 255,255,255,255]), 8)!;
    try { await writeFile(join(dir, "steps.png"), image.encodeToBytes()!); } finally { image.delete(); }
    await writeFile(join(dir, "effect.vsksl"), `
      float4 valle_main(float2 uv) {
        float3 sum = float3(0.0);
        for (int i = 0; i < 2; ++i) { sum += tap(basis * uv * 2.0 + offset.xy).rgb * 0.5; }
        if (enabled) return float4(sum + tint.rgb * amount, 1.0);
        return float4(0.0);
      }
      float4 tap(float2 uv) { return sample_steps(uv); }
    `);
    await writeFile(join(dir, "effect.motion.tsx"), `
      export const composition = { width: 64, height: 64, duration: 3 };
      export const controls={assets:{effect:asset({kind:"shader"}),image:asset({kind:"image"})}};
      export default function T(ctx) { return <Scene style={{width:64,height:64}}>
        <ShaderLayer source="asset://effect" inputs={{steps:"asset://image"}}
          uniforms={{basis:[1,0,0,1],offset:[ctx.seconds*0.1,0,0],amount:ctx.seconds*0.05,tint:"#ff000000",enabled:true}}
          style={{width:64,height:64}}><View style={{width:64,height:64,backgroundColor:"#fff"}}/></ShaderLayer>
      </Scene>; }
    `);
    const fieldSource = await readFile(join(dir, "effect.vsksl"), "utf8");
    const fieldMotion = await readFile(join(dir, "effect.motion.tsx"), "utf8");
    const transformPoint = (x: number, y: number): [number, number] => {
      const angle = 37 * Math.PI / 180;
      x = (x - 16) * 1.25; y = (y - 16) * 0.75;
      return [32 + x * Math.cos(angle) - y * Math.sin(angle), 32 + x * Math.sin(angle) + y * Math.cos(angle)];
    };
    const transformedEdges: Edge[] = [[0,0,32,0], [32,0,32,32], [32,32,0,32], [0,32,0,0], [16,0,16,32]]
      .map(([x0,y0,x1,y1]) => [...transformPoint(x0!,y0!), ...transformPoint(x1!,y1!)]);
    const contents = [
      { name: "nested-offscreen", body: `<ShaderLayer source="asset://effect" style={{position:"absolute",left:-16,top:16,width:32,height:32}}>
          <ShaderLayer source="asset://effect" style={{width:32,height:32}}><View style={{width:32,height:32,backgroundColor:"#fff"}}/></ShaderLayer>
        </ShaderLayer>`, source: "float4 sum = float4(0.0); for (int i = 0; i < 64; ++i) { sum += sampleContent(uv) / 64.0; } return sum;", points: [[8,32,255,255,255], [24,32,0,0,0]], samples: (32*32+16*32)*64 },
      { name: "transformed", body: `<ShaderLayer source="asset://effect" style={{position:"absolute",left:16,top:16,width:32,height:32,transform:"rotate(37deg) scale(1.25,0.75)"}}>
          <View style={{position:"absolute",width:16,height:32,backgroundColor:"#f00"}}/>
          <View style={{position:"absolute",left:16,width:16,height:32,backgroundColor:"#00f"}}/>
        </ShaderLayer>`, source: "return sampleContent(uv);", points: [[24,26,255,0,0], [40,38,0,0,255], [8,8,0,0,0]], edges: transformedEdges },
      { name: "offscreen", body: `<ShaderLayer source="asset://effect" style={{position:"absolute",left:-32,top:16,width:64,height:32}}>
          <View style={{position:"absolute",width:32,height:32,backgroundColor:"#f00"}}/>
          <View style={{position:"absolute",left:32,width:32,height:32,backgroundColor:"#00f"}}/>
        </ShaderLayer>`, source: "return sampleContent(uv - float2(0.5,0.0));", points: [[16,32,255,0,0], [48,32,0,0,0]], samples: 32*32 },
      { name: "sparse", body: `<ShaderLayer source="asset://effect" style={{position:"absolute",left:8,top:8,width:48,height:48}}>
          <View style={{position:"absolute",left:8,top:8,width:8,height:8,backgroundColor:"#f00"}}/>
        </ShaderLayer>`, source: "return sampleContent(uv);", points: [[20,20,255,0,0], [32,32,0,0,0]] },
      { name: "empty", body: `<ShaderLayer source="asset://effect" style={{position:"absolute",left:8,top:8,width:48,height:48}}/>`,
        source: "if (uv.x < 0.5) return float4(1.0); return float4(0.0);", points: [[16,32,255,255,255], [48,32,0,0,0]], samples: 0 },
      { name: "padding", body: `<ShaderLayer source="asset://effect" style={{position:"absolute",left:24,top:24,width:16,height:16}}>
          <View style={{width:16,height:16,backgroundColor:"#fff"}}/>
        </ShaderLayer>`, source: "return sampleContent(uv + float2(0.5,0.0));", points: [[20,32,255,255,255], [28,32,255,255,255], [40,32,0,0,0], [12,32,0,0,0]], samples: 36*26 },
      { name: "scene3d", body: `<ShaderLayer source="asset://effect" style={{position:"absolute",left:8,top:8,width:48,height:48}}>
          <Scene3D key="model" camera={{position:[ctx.seconds*0.03,0,3],target:[0,0,0],near:0.1+ctx.seconds*0.01,fov:45}} style={{width:48,height:48}}>
            <Mesh key="part" src="asset://model" rotateY={ctx.seconds*20} nodes={[{id:0,rotation:[0,0,ctx.seconds*5],scale:[-1,1,1]}]} material={{type:"unlit",color:ctx.seconds<1?"#ff0000":"#0000ff"}}/>
          </Scene3D>
        </ShaderLayer>`, source: "float4 c = sampleContent(uv); return float4(c.b,c.g,c.r,c.a);", points: [[32,32,0,0,255],[10,10,0,0,0]], samples: 48*48 },
    ];
    const failures: string[] = [];
    for (const [sampling, wrap] of [["optional-middle", "clamp"], ["optional-empty", "clamp"], ["data-nearest", "mirror"], ["data-linear", "repeat"], ["data-precision", "clamp"], ["nearest", "mirror"], ["linear", "repeat"], ...contents.map(c => [c.name, "clamp"])]) {
      const content = contents.find(c => c.name === sampling);
      const scene3d = sampling === "scene3d";
      const caseAssets = [...assets, ...(scene3d ? ["--asset","model=model.glb"] : [])];
      if (scene3d) await writeFile(join(dir,"model.glb"),await readFile(join(root,"crates/valle-motion/tests/fixtures/scene3d/triangle.glb")));
      const data = sampling!.startsWith("data-");
      const precision = sampling === "data-precision";
      const optional = sampling!.startsWith("optional-");
      const filter = sampling === "data-linear" ? "linear" : data ? "nearest" : sampling;
      if (!data) await writeFile(join(dir, "steps.png"), numericPng([0,0,0,255, 255,255,255,255], 8));
      if (data) await writeFile(join(dir, "steps.png"), numericPng(precision ? [32768,0,0,0, 32769,0,0,65535] : [128,64,32,0, 64,128,192,255], precision ? 16 : 8));
      await writeFile(join(dir, "effect.vsksl"), content ? `float4 valle_main(float2 uv) { ${content.source} }` : fieldSource);
      await writeFile(join(dir, "effect.motion.tsx"), content ? `
        export const composition = { width: 64, height: 64, duration: 3 };
        export const controls={assets:{effect:asset({kind:"shader"}),image:asset({kind:"image"})${scene3d?',model:asset({kind:"model3d"})':''}}};
        export default function T(ctx) { return <Scene style={{width:64,height:64,backgroundColor:"#000"}}>${content.body}</Scene>; }
      ` : fieldMotion);
      if (data) {
        await writeFile(join(dir, "effect.vsksl"), precision
          ? "float4 valle_main(float2 uv) { float n = (sample_steps(float2(0.75,0.5)).r - sample_steps(float2(0.25,0.5)).r) * 65535.0; return float4(n,n,n,1.0); }"
          : "float4 valle_main(float2 uv) { float2 p = float2(uv.x * 2.0 - 0.5, 0.5); if (uv.y < 0.5) return float4(sample_steps(p).rgb, 1.0); return sample_color(p); }");
        await writeFile(join(dir, "effect.motion.tsx"), `
          export const composition = { width: 64, height: 64, duration: 3 };
          export const controls={assets:{effect:asset({kind:"shader"}),image:asset({kind:"image"})}};
          export default function T(){ return <Scene style={{width:64,height:64,backgroundColor:"#000"}}>
            <ShaderLayer source="asset://effect" inputs={{steps:"asset://image",color:"asset://image"}} style={{width:64,height:64}}/>
          </Scene>; }
        `);
      }
      if (optional) {
        await writeFile(join(dir, "effect.vsksl"), "float4 valle_main(float2 uv) { return sample_before(uv) + sample_steps(uv) + sample_after(uv) + float4(0.0,0.25,0.0,1.0); }");
        await writeFile(join(dir, "effect.motion.tsx"), `
          export const composition = { width: 64, height: 64, duration: 3 };
          export const controls={assets:{effect:asset({kind:"shader"}),image:asset({kind:"image"})}};
          export default function T(){ return <Scene style={{width:64,height:64}}>
            <ShaderLayer source="asset://effect" ${sampling === "optional-middle" ? 'inputs={{steps:"asset://image"}}' : ''} style={{width:64,height:64}}/>
          </Scene>; }
        `);
      }
      await writeFile(join(dir, "effect.shader.json"), JSON.stringify({
        name: "sample-field", entry: "effect.vsksl", inputs: content ? [] : optional
          ? [{ name: "before", kind: "color", required: false, sampling: "linear", wrap: "repeat" }, { name: "steps", kind: "color", required: false, sampling: "nearest", wrap: "clamp" }, { name: "after", kind: "data", required: false, sampling: "nearest", wrap: "mirror" }]
          : data
          ? [{ name: "steps", kind: "data", required: true, sampling: filter, wrap }, { name: "color", kind: "color", required: true, sampling: filter, wrap }]
          : [{ name: "steps", required: true, sampling, wrap }],
        uniforms: content || data || optional ? [] : [
          { name: "basis", type: "float2x2", required: true, min: -1, max: 1 },
          { name: "offset", type: "float3", required: true, min: -1, max: 1 },
          { name: "amount", type: "float", required: true, min: 0, max: 1 },
          { name: "tint", type: "color", required: true }, { name: "enabled", type: "bool", required: true },
        ], output: { colorSpace: "linear-srgb", alphaMode: "straight", allowTransparent: true, padding: sampling === "padding" ? [8,4,12,6] : [0,0,0,0] }, budget: "local",
      }));
      // Read the same verified package used by Native delivery through the existing local host.
      const server = spawn(["motion", "studio", "effect.motion.tsx", ...caseAssets, "--port", "0", "--web-assets-dir", join(root, "web/dist")]);
      let engine: ProductEngine | undefined;
      let executor: CanvasKitExecutor | undefined;
      const surface = ck.MakeSurface(64, 64)!;
      try {
        const reader = server.stdout.getReader();
        let line = "";
        try {
          while (!line.includes("\n")) {
            const part = await reader.read();
            if (part.done) throw new Error(await new Response(server.stderr).text());
            line += new TextDecoder().decode(part.value);
          }
        } finally { reader.releaseLock(); }
        const ready = JSON.parse(line.split("\n")[0]!);
        if (!ready.url) throw new Error(JSON.stringify(ready));
        const config = await (await fetch(new URL("/config.json", ready.url))).json() as Record<string, string>;
        const open = () => {
          const engine = new ProductEngine();
          const receipt = JSON.parse(engine.open_fixed_package(config.fixedPackageManifestJson!, config.timelineJson!, config.resourceManifestJson!, config.verifiedBindingBundleJson!));
          return { engine, renderId: receipt.renderId as string };
        };
        const opened = open(); engine = opened.engine;
        let cpuOutputCalls = 0;
        executor = new CanvasKitExecutor(ck, {
          transform_srgb_preview_pixels(pixels, opaque) {
            cpuOutputCalls += 1;
            engine!.transform_srgb_preview_pixels(pixels, opaque);
          },
          pack_motion_glass_uniforms: engine.pack_motion_glass_uniforms.bind(engine),
          pack_motion_glass_foreground_uniforms: engine.pack_motion_glass_foreground_uniforms.bind(engine),
        });
        const renderWeb = async (engine: ProductEngine, renderId: string, frame: number, transparent = false) => {
          const ticket = engine.evaluate_prepare_preview(renderId, BigInt(frame), 64, 64, transparent);
          const owned: Array<{ delete(): void }> = [];
          try {
            const requests = await decodePackedAbi(engine.resource_requests(ticket), RESOURCE_REQUESTS_ABI) as any[];
            const objects = new Map<number, CanvasKitExternalObject>();
            for (const request of requests) {
              if (request.expected.kind === "runtimeShader") {
                const bytes = engine.compiled_resource_bytes(renderId, "runtime-shader", request.key.content, request.key.interpretation.abi_digest);
                objects.set(request.handle, { key: request.key, kind: "runtimeShader", bytes });
              } else if (request.expected.kind === "visualFrame") {
                const image = ck.MakeImageFromEncoded(await readFile(join(dir, "steps.png")))!;
                owned.push(image);
                objects.set(request.handle, { key: request.key, kind: "visual", image });
              } else if (request.expected.kind === "dataTexture") {
                const { width, height } = request.expected.extent;
                const pixels = engine.decode_shader_data_texture(request.key.content, await readFile(join(dir, "steps.png")), width, height);
                const image = ck.MakeImage({ width: width * 2, height: height * 4, colorType: ck.ColorType.Alpha_8,
                  alphaType: ck.AlphaType.Premul, colorSpace: ck.ColorSpace.SRGB }, pixels, width * 2)!;
                owned.push(image);
                objects.set(request.handle, { key: request.key, kind: "dataTexture", image });
              } else if (request.expected.kind === "scene3d") {
                const payload = Uint8Array.from(request.payload.canonical_request as number[]);
                const needs = JSON.parse(engine.scene3d_resource_needs_json(payload));
                for (const digest of needs.models) engine.register_scene3d_model(digest,engine.compiled_resource_bytes(renderId,"model3d-bytes",digest));
                const pixels = engine.render_scene3d_request(request.key.content,request.key.interpretation.topology_digest,payload);
                const width = engine.scene3d_frame_width(request.key.content), height = engine.scene3d_frame_height(request.key.content);
                const image = ck.MakeImage({...info,width,height,alphaType:ck.AlphaType.Premul},pixels,width*4)!;
                owned.push(image);
                objects.set(request.handle,{key:request.key,kind:"scene3d",image});
              } else throw new Error(`unexpected shader dependency ${request.expected.kind}`);
            }
            if (optional) {
              expect(requests.filter(r => r.expected.kind === "visualFrame").length).toBe(sampling === "optional-middle" ? 1 : 0);
              expect(requests.filter(r => r.expected.kind === "dataTexture").length).toBe(0);
            }
            if (data) {
              expect(requests.filter(r => r.expected.kind === "dataTexture").length, JSON.stringify(requests)).toBe(1);
              expect(requests.filter(r => r.expected.kind === "visualFrame").length).toBe(1);
            }
            engine.lower_canvas_kit(ticket, 64n * 1024n * 1024n, 128n * 1024n * 1024n);
            engine.bind(ticket, 1n);
            const schedule = await decodePackedAbi(engine.bound_schedule_bytes(ticket), BOUND_PROGRAM_SCHEDULES_ABI) as any;
            if (content?.samples !== undefined) expect(schedule.shaderWork.samples, sampling).toBe(content.samples);
            if (data && frame === 60) {
              const surfaceBudget = BigInt(schedule.estimatedPeakSurfaceBytes);
              const limited = engine.evaluate_prepare_preview(renderId, BigInt(frame), 64, 64, false);
              try {
                expect(() => {
                  engine.lower_canvas_kit(limited, surfaceBudget, surfaceBudget);
                  engine.bind(limited, 1n);
                  engine.bound_schedule_bytes(limited);
                }).toThrow("bytes");
              } finally { engine.release_ticket(limited); }
            }
            await executor!.execute(engine.plan_template_bytes(ticket), engine.binding_bytes(ticket), engine.bound_schedule_bytes(ticket), { generation: 1n, objects }, { surface });
            return Uint8Array.from(surface.getCanvas().readPixels(0, 0, info)!);
          } finally { owned.forEach(v => v.delete()); engine.release_ticket(ticket); }
        };
        let first: Uint8Array<ArrayBuffer> | undefined;
        let firstNative: Uint8Array<ArrayBuffer> | undefined;
        for (const [request, frame] of (content && !scene3d ? [60, 60] : [60, 0, 30, 60]).entries()) {
          const path = join(dir, `native-${sampling}-${request}.png`);
          const nativeRun = spawn(["motion", "render", "effect.motion.tsx", ...caseAssets, "--frame", String(frame), "--backend", nativeBackend, "-o", path]);
          const [stdout, stderr, code] = await Promise.all([new Response(nativeRun.stdout).text(), new Response(nativeRun.stderr).text(), nativeRun.exited]);
          if (code !== 0) throw new Error(`${stdout}\n${stderr}`);
          expect(JSON.parse(stdout).delivery.backend).toBe(nativeBackend);
          const image = ck.MakeImageFromEncoded(await readFile(path))!;
          const native = Uint8Array.from(image.readPixels(0, 0, info)!);
          image.delete();
          if (frame === 60) {
            if (firstNative) expect(native, `${nativeBackend}/${sampling}: repeated frame 60`).toEqual(firstNative);
            else firstNative = native;
          }
          const web = await renderWeb(engine, opened.renderId, frame);
          const difference = pixelDifference(web, native, 64, content?.edges);
          const label = `${nativeBackend}/${sampling}/${wrap}, frame ${frame}`;
          expect(difference.interior, label).toBeLessThanOrEqual(2);
          if (content?.edges) {
            // At most 1/8 coverage error inside the known AA band and 0.2% over
            // the frame. Uniform color errors and geometry shifts still fail.
            expect(difference.edge, label).toBeLessThanOrEqual(0.125);
            expect(difference.mean, label).toBeLessThanOrEqual(0.002);
            if (request === 0) {
              const tinted = Uint8Array.from(web);
              tinted[(26 * 64 + 24) * 4 + 1] += 9;
              expect(pixelDifference(tinted, native, 64, content.edges).interior).toBeGreaterThan(2);
              const shifted = Uint8Array.from(web);
              for (let y = 0; y < 64; y++) shifted.set(web.subarray(y * 256, y * 256 + 252), y * 256 + 4);
              const shift = pixelDifference(shifted, native, 64, content.edges);
              expect(shift.interior > 2 || shift.edge > 0.125 || shift.mean > 0.002).toBe(true);
            }
          }
          for (const [x, y, ...rgb] of content?.points ?? []) {
            const at = (y! * 64 + x!) * 4;
            const expectedRgb = scene3d && x===32 ? (frame===0 ? [0,0,255] : [255,0,0]) : rgb;
            for (let c = 0; c < 3; c++) expect(Math.abs(web[at+c]! - expectedRgb[c]!), `${sampling} (${x},${y}) channel ${c}`).toBeLessThanOrEqual(2);
          }
          if (optional) {
            for (const x of [8,48]) {
              const expected = sampling === "optional-middle" && x === 48 ? [255,255,255] : [0,137,0];
              const at = (16 * 64 + x) * 4;
              for (let c = 0; c < 3; c++) expect(Math.abs(web[at+c]! - expected[c]!)).toBeLessThanOrEqual(2);
            }
          }
          if (precision) expect(Array.from(web.slice((16 * 64 + 16) * 4, (16 * 64 + 16) * 4 + 3))).toEqual([255,255,255]);
          if (sampling === "data-nearest") {
            for (const [x,y,r,g,b] of [[16,16,188,137,99], [48,16,137,188,225], [16,48,0,0,0], [48,48,64,128,192]]) {
              const at = (y! * 64 + x!) * 4;
              for (const [c, expected] of [r,g,b].entries()) expect(Math.abs(web[at+c]! - expected!)).toBeLessThanOrEqual(2);
            }
          }
          if (frame === 60) { if (first) expect(web).toEqual(first); else first = web; }
        }
        expect(cpuOutputCalls).toBeGreaterThan(0);
        const opaqueCalls = cpuOutputCalls;
        await renderWeb(engine, opened.renderId, 60, true);
        expect(cpuOutputCalls, "coverage output must retain the SkSL conversion").toBe(opaqueCalls);
        if (scene3d) {await rm(join(dir,"model.glb"));await rm(join(dir,"effect.vsksl"));}
        const fresh = open();
        try { expect(await renderWeb(fresh.engine, fresh.renderId, 60)).toEqual(first!); } finally { fresh.engine.free(); }
      } catch (error) {
        failures.push(`${sampling}/${wrap}: ${error instanceof Error ? error.message : String(error)}`);
      } finally {
        server.kill(); await server.exited;
        executor?.dispose(); engine?.free(); surface.delete();
      }
    }
    expect(failures, `${nativeBackend}/CanvasKit Shader parity`).toEqual([]);
  } finally { await rm(dir, { recursive: true, force: true }); }
}, 180_000);
