// Manual browser acceptance host. It serves Studio and frozen fixtures; it never launches Valle
// or FFmpeg. Open /studio, export with the real toolbar, then /__test/checks for additional checks.
import { appendFile, mkdir } from "node:fs/promises";
import path from "node:path";
import { initSync, compile_motion_jsx, prepare_preview_package } from "../packages/engine/generated/web/valle_engine.js";
import { compileMotionJsxWithWasm, preparePreviewPackageWithWasm, type PreviewResourceInput } from "../packages/engine/src/compiler.ts";
import type { Timeline } from "../packages/engine/src/index.ts";
import { STUDIO_HOST_PROTOCOL_VERSION } from "../packages/engine/src/generated/protocol.ts";

const web = path.resolve(import.meta.dir, "..");
const dist = path.join(web, "dist");
const evidence = path.resolve(process.env.VALLE_BROWSER_EXPORT_EVIDENCE ?? "/private/tmp/valle-browser-export");
await mkdir(evidence, { recursive: true });
initSync({ module: await Bun.file(path.join(web, "packages/engine/generated/web/valle_engine_bg.wasm")).arrayBuffer() });
const hash = (bytes: Uint8Array | string) => `sha256:${new Bun.CryptoHasher("sha256").update(bytes).digest("hex")}`;
const sourcePath = "/fixtures/scene.motion.tsx";
const source = `export const composition={width:640,height:360,fps:30,duration:3};
export const controls={props:{size:number({default:60,min:20,max:160})}};
export default function SceneClip(ctx,props){return <Scene style={{width:640,height:360,backgroundColor:"#102030"}}>
  <View key="box" style={{position:"absolute",left:30+ctx.progress*450,top:140,width:props.size,height:props.size,backgroundColor:"#67e8f9",borderRadius:12}}/>
  <View style={{position:"absolute",left:30,top:300,width:580*ctx.progress,height:8,backgroundColor:"#ffffff"}}/>
</Scene>;}`;

function tone() {
  const rate = 48_000, samples = rate * 3;
  const bytes = new Uint8Array(44 + samples * 4), view = new DataView(bytes.buffer);
  const text = (at: number, value: string) => bytes.set(new TextEncoder().encode(value), at);
  text(0, "RIFF"); view.setUint32(4, bytes.length - 8, true); text(8, "WAVEfmt ");
  view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 2, true);
  view.setUint32(24, rate, true); view.setUint32(28, rate * 4, true); view.setUint16(32, 4, true);
  view.setUint16(34, 16, true); text(36, "data"); view.setUint32(40, samples * 4, true);
  for (let sample = 0; sample < samples; sample++) {
    const frequency = [440, 880, 1760][Math.floor(sample / rate)]!;
    const value = Math.round(Math.sin(sample * frequency * 2 * Math.PI / rate) * 12_000);
    view.setInt16(44 + sample * 4, value, true); view.setInt16(46 + sample * 4, value, true);
  }
  return bytes;
}
const wav = tone();
const descriptor = { duration: "3/1", timeBase: "1/48000", presentationIndexDigest: hash("fixture:48000Hz:144000samples"),
  sampleRate: 48_000, channelLayout: "stereo", audioStream: 0 };
const audioResource: PreviewResourceInput = { id: "resource:tone", entry: { kind: "audio", digest: hash(wav), descriptor },
  facts: { kind: "audio", descriptor, temporalFootprint: { pastSamples: 0, futureSamples: 0 } } };
const runtimeAssets = { engine: { glue: "/runtime/engine/valle_engine.js", wasm: "/runtime/engine/valle_engine_bg.wasm" },
  canvasKit: { glue: "/runtime/canvaskit/canvaskit.js", wasm: "/runtime/canvaskit/canvaskit.wasm" },
  workers: { productFrame: "/runtime/workers/product-frame.js" } };

function fixture(name: string, videoBytes?: Uint8Array) {
  const motion = name === "motion";
  const fractional = name === "fractional";
  const hasAudio = name === "av" || name === "short-av" || name === "video-av";
  const isVideo = name === "video" || name === "video-av";
  const videoDescriptor = { duration: "3/1", timeBase: "1/30", presentationIndexDigest: hash("fixture:30fps:90frames"),
    width: 640, height: 360, orientation: "identity", videoStream: 0, audioStream: null,
    // CanvasSource's browser-generated fixture carries an sRGB transfer (iec61966-2-1),
    // with BT.709 primaries/matrix. Admission facts must match the encoded media.
    color: { primaries: "bt709", transfer: "srgb", matrix: "bt709", fullRange: false } };
  const resources: PreviewResourceInput[] = [
    ...(isVideo ? [{ id: "resource:video", entry: { kind: "video", digest: hash(videoBytes!), descriptor: videoDescriptor },
      facts: { kind: "video", descriptor: videoDescriptor, temporalFootprint: { pastFrames: 0, futureFrames: 0 } } }] : []),
    ...(hasAudio ? [audioResource] : []),
  ];
  const duration = name === "long" ? 30 : fractional ? 1.001 : name === "short-av" ? 1 / 30 : 3;
  const author: Timeline = { canvas: { width: name === "odd" ? 641 : 640, height: 360, fps: fractional ? "30000/1001" : 30 },
    resources: { ...(motion ? { scene: sourcePath } : {}), ...(hasAudio ? { tone: "/fixtures/tone.wav" } : {}),
      ...(isVideo ? { video: "/fixtures/source.mp4" } : {}) },
    tracks: { visual: [{ clips: motion ? [{ kind: "motion", component: "scene", start: 0, duration }]
      : isVideo ? [{ kind: "video", src: "video", start: 0, duration }]
      : fractional || name === "short-av" ? [{ kind: "solid", color: "#20c060", start: 0, duration }]
      : [{ kind: "solid", color: "#e03030", start: 0, duration: duration / 3 },
        { kind: "solid", color: "#20c060", start: duration / 3, duration: duration / 3 },
        { kind: "solid", color: "#3070e0", start: duration * 2 / 3, duration: duration / 3 }] }],
      ...(hasAudio ? { audio: [{ clips: [{ src: "tone", start: 0, duration, gain: 0.5 }] }] } : {}) } };
  const compiled = motion ? compileMotionJsxWithWasm({ compile_motion_jsx }, source) : null;
  const prepared = preparePreviewPackageWithWasm({ prepare_preview_package }, { authorTimeline: author,
    motionInstances: compiled ? [{ clipPath: "/tracks/visual/0/clips/0", artifact: compiled.artifact,
      artifactDigest: compiled.artifactDigest, fonts: [] }] : [], resourceInputs: resources });
  const render = { fixedPackageManifestJson: prepared.fixedPackageManifestJson, timelineJson: prepared.timelineJson,
    timeline: JSON.parse(prepared.timelineJson), resourceManifestJson: prepared.resourceManifestJson,
    resourceManifest: JSON.parse(prepared.resourceManifestJson), verifiedBindingBundleJson: prepared.verifiedBindingBundleJson };
  const assets = [...(hasAudio ? [{ id: "resource:tone", url: "/fixtures/tone.wav" }] : []),
    ...(isVideo ? [{ id: "resource:video", url: "/fixtures/source.mp4" }] : [])];
  return { author, prepared, render, assets, compiled, resourceInputs: resources,
    input: { ...render, assets, runtimeAssets, assetBaseUrl: "/fixtures/" } };
}
const fixtures = new Map(["motion", "av", "short-av", "fractional", "long", "odd"].map((name) => [name, fixture(name)]));
const selected = process.env.VALLE_BROWSER_EXPORT_FIXTURE ?? "av";
if (selected === "video" || selected === "video-av") {
  fixtures.set(selected, fixture(selected, new Uint8Array(await Bun.file(path.join(evidence, "av.mp4")).arrayBuffer())));
}
const current = fixtures.get(selected);
if (!current) throw new Error(`Unknown fixture ${selected}`);
let hostLocked = false;
const client = await Bun.build({ entrypoints: [path.join(web, "tests/browser-export-client.ts")], target: "browser", format: "esm" });
if (!client.success) throw new Error(client.logs.map((log) => log.message).join("\n"));
const boot = { protocolVersion: STUDIO_HOST_PROTOCOL_VERSION,
  session: { kind: "timeline-file", input: `/fixtures/${selected}.timeline.json`, token: "browser-export-test" },
  capabilities: { saveTimeline: false, editProject: false, editMotionProps: false, writeMotionSource: true },
  runtime: { assetBaseUrl: "/fixtures/", fontUrls: [], assetUrls: { engineGlue: runtimeAssets.engine.glue,
    engineWasm: runtimeAssets.engine.wasm, canvasKitGlue: runtimeAssets.canvasKit.glue,
    canvasKitWasm: runtimeAssets.canvasKit.wasm, productFrameWorker: runtimeAssets.workers.productFrame,
    studioCompileWorker: "/runtime/workers/studio-compile.js" } } };
const hostPaths = new Set(["/studio/boot.json", "/source/files", "/timeline/get", "/timeline/media-facts"]);
const server = Bun.serve({ hostname: "127.0.0.1", port: Number(process.env.PORT ?? 9537), async fetch(request) {
  const url = new URL(request.url), route = url.pathname;
  await appendFile(path.join(evidence, "requests.jsonl"), `${JSON.stringify({ at: new Date().toISOString(), method: request.method, route, hostLocked })}\n`);
  if (route === "/__test/lock-host" && request.method === "POST") { hostLocked = true; return Response.json({ hostLocked }); }
  if (route === "/__test/unlock-host" && request.method === "POST") { hostLocked = false; return Response.json({ hostLocked }); }
  if (hostLocked && hostPaths.has(route)) return new Response("Host APIs disabled during export acceptance", { status: 503 });
  if (route === "/studio/boot.json") return Response.json(boot);
  if (route === "/timeline/get") return Response.json({ timeline: current.author, timelineJson: JSON.stringify(current.author),
    timelineRevision: { revision: 1, parentRevision: null, createdAt: "2026-10-08T00:00:00Z", actor: "test", cause: { type: "genesis" }, intent: null },
    motionSourceMetadata: current.prepared.motionSourceMetadata, render: current.render, assets: current.assets,
    motion: { structures: [] }, motionInstances: current.compiled ? [{ clipPath: "/tracks/visual/0/clips/0",
      artifact: current.compiled.artifact, sourceMap: current.compiled.sourceMap }] : [] });
  if (route === "/source/files") {
    const drafts = request.method === "POST" ? (await request.json()).drafts ?? {} : {};
    return Response.json({ files: selected === "motion" ? [{ path: sourcePath, status: "ok", text: drafts[sourcePath] ?? source, baseDigest: hash(source) }] : [] });
  }
  if (route === "/timeline/media-facts") return Response.json({ status: "ok", resources: current.resourceInputs, assets: current.assets, inputDependencies: {} });
  if (route === "/events") return new Response(null, { status: 204 });
  if (route === "/fixtures/tone.wav") return new Response(wav, { headers: { "content-type": "audio/wav" } });
  if (route === "/fixtures/source.mp4") return new Response(Bun.file(path.join(evidence, "av.mp4")), { headers: { "content-type": "video/mp4" } });
  if (route === "/fixtures/offset-bframes.mp4") return new Response(Bun.file(path.join(web, "tests/fixtures/video-offset-bframes.mp4")), { headers: { "content-type": "video/mp4" } });
  if (route === "/__test/input.json") {
    const name = url.searchParams.get("fixture") ?? selected;
    if (name === "video" || name === "video-av") fixtures.set(name, fixture(name, new Uint8Array(await Bun.file(path.join(evidence, "av.mp4")).arrayBuffer())));
    const item = fixtures.get(name);
    return item ? Response.json(item.input) : new Response("Unknown fixture", { status: 404 });
  }
  if (route === "/__test/checks") return new Response('<!doctype html><meta charset="utf-8"><title>Browser export checks</title><style>body{font:16px system-ui;margin:40px;background:#101113;color:#e8eaed}pre{white-space:pre-wrap}</style><h1>Browser export checks</h1><pre id="status">Running…</pre><script type="module" src="/__test/client.js"></script>', { headers: { "content-type": "text/html" } });
  if (route === "/__test/client.js") return new Response(client.outputs[0], { headers: { "content-type": "application/javascript" } });
  if (request.method === "POST" && /^\/__test\/artifacts\/[a-z0-9-]+\.(mp4|json)$/.test(route)) {
    await Bun.write(path.join(evidence, path.basename(route)), await request.arrayBuffer());
    return Response.json({ saved: path.basename(route) });
  }
  const relative = route === "/studio" || route === "/" ? "apps/studio/index.html" : route.slice(1);
  const file = path.resolve(dist, relative);
  if (!file.startsWith(`${dist}${path.sep}`)) return new Response("Not found", { status: 404 });
  const asset = Bun.file(file);
  return await asset.exists() ? new Response(asset) : new Response("Not found", { status: 404 });
} });
console.log(JSON.stringify({ url: `${server.url}studio`, checks: `${server.url}__test/checks`, selected, evidence, nativeProcesses: 0 }));
