// Real browser acceptance of full Scene3D rasterization in the ordinary export configuration.
import { mkdtemp, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { CanvasKitGpuHost } from "../packages/engine/src/executor/webgl/gpu-state.ts";
import {
  browserLaunchCommand,
  resolveBrowser,
  terminateBrowser,
  waitForUnexpectedBrowserExit,
} from "./parity/lib/browser.ts";

const root = resolve(import.meta.dir, "../..");
const cli = process.env.VALLE_TEST_CLI ?? join(root, "target/debug/valle");
const runtime = process.env.VALLE_WEB_RUNTIME_DIR ?? join(root, "web/dist");
const browser = resolveBrowser();
if (!browser) {
  if (process.env.VALLE_WEB_PLAYER_REQUIRE_BROWSER || process.env.VALLE_WEB_PARITY_REQUIRE_BROWSER)
    throw new Error("browser unavailable");
  console.log(JSON.stringify({ status: "skipped", reason: "browser unavailable" }));
  process.exit(0);
}
const evidence = await mkdtemp(join(tmpdir(), "valle-scene3d-browser-"));
const [width, height] = process.argv.includes("--4k") ? [3840, 2160] : [384, 256];
const engine = join(root, "web/packages/engine");
const { default: init } = (await import(Bun.resolveSync("canvaskit-wasm", engine))) as {
  default: (o: { locateFile(): string }) => Promise<CanvasKitGpuHost["CanvasKit"]>;
};
const ck = await init({
  locateFile: () => Bun.resolveSync("canvaskit-wasm/bin/canvaskit.wasm", engine),
});
const rgba = new Uint8Array(64 * 32 * 4);
for (let y = 0; y < 32; y++)
  for (let x = 0; x < 64; x++)
    rgba.set([64 + x * 3, 64 + y * 5, 192, (x >> 2) % 2 ? 255 : 48], (y * 64 + x) * 4);
const image = ck.MakeImage(
  {
    width: 64,
    height: 32,
    colorType: ck.ColorType.RGBA_8888,
    alphaType: ck.AlphaType.Unpremul,
    colorSpace: ck.ColorSpace.SRGB,
  },
  rgba,
  64 * 4,
)!;
try {
  await Bun.write(join(evidence, "map.png"), image.encodeToBytes()!);
} finally {
  image.delete();
}

function source(resources: boolean): string {
  const textures = resources
    ? ',textures:{baseColor:"asset://map",metallicRoughness:{src:"asset://map",wrapU:"repeat",wrapV:"mirror",mipmap:"nearest"},normal:"asset://map",occlusion:"asset://map",emissive:"asset://map"}'
    : "";
  return `export const composition={width:${width},height:${height},fps:30,duration:2};
export const controls={assets:{${resources ? 'model:asset({kind:"model3d"}),map:asset({kind:"image"}),sky:asset({kind:"environment"})' : ""}}};
const BALL=lathe(path("M 0 -1 C 0.5523 -1 1 -0.5523 1 0 C 1 0.5523 0.5523 1 0 1"),{segments:8});
export default function Main(){const time=useCurrentTime();return <Scene style={{backgroundColor:"#102030"}}>
<Scene3D key="gpu" style={{width:${width},height:${height}}} camera={{position:[time.seconds*0.05,0,5],target:[0,0,0],fov:45,depthOfField:{focusDistance:5,maxBlurRadius:8}}}
 pbr={{shadows:true,toneMapping:"aces",exposure:0.6+time.seconds*0.3${resources ? ',environment:{src:"asset://sky",rotation:time.seconds*90,intensity:0.3,background:true}' : ""}}}>
 <Mesh key="main" geometry={BALL} rotateY={time.seconds*90} position={[-0.4,0,0]}
  material={{type:"pbr",color:"#cf7338",metallic:0.1+time.seconds*0.4,roughness:0.1+time.seconds*0.3,normalScale:0.3,emissive:"#101a22",emissiveIntensity:0.2${textures}}}/>
 <Mesh key="glass" geometry={BALL} scale={[0.45,0.45,0.45]} position={[0.6,0,1.2]}
  material={{type:"pbr",alphaMode:"blend",color:"#45aadd66",roughness:0.2${textures}}}/>
 ${resources ? '<Mesh key="model" src="asset://model" position={[1.5,0,-0.8]} rotateY={time.seconds*20} scale={[-1,1,1]} material={{type:"lambert",doubleSided:true,alphaMode:"mask",alphaCutoff:0.4,textures:{baseColor:"asset://map"}}}/>' : ""}
 <AmbientLight intensity={0.2}/><HemisphereLight skyColor="#b0c0ff" groundColor="#231507" intensity={0.2}/>
 <DirectionalLight direction={[0.8,0.2,1]} intensity={1.6}/><DirectionalLight direction={[-1,0.2,1]} intensity={0.8}/>
</Scene3D></Scene>;}`;
}

async function deadline<T>(work: Promise<T>, lastActivity?: () => number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const started = Date.now();
  try {
    return await Promise.race([
      work,
      new Promise<never>((_, reject) => {
        const check = () => {
          const now = Date.now();
          const remaining = Math.min(
            120_000 - (now - (lastActivity?.() ?? started)),
            300_000 - (now - started),
          );
          if (remaining <= 0) reject(new Error("Scene3D browser acceptance timed out"));
          else timer = setTimeout(check, remaining);
        };
        timer = setTimeout(check, 120_000);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

const cases = process.argv
  .find((arg) => arg.startsWith("--cases="))
  ?.slice(8)
  .split(",") ?? ["unlit", "morph", "skin", "clip", "procedural", "assets"];
if (
  cases.some((name) => !["unlit", "morph", "skin", "clip", "procedural", "assets"].includes(name))
)
  throw new Error("Unknown Scene3D case");
for (const name of cases) {
  const animated = name === "morph" || name === "skin";
  const resources = name === "assets",
    directory = join(evidence, name);
  await mkdir(directory);
  const input = join(directory, "scene.motion.tsx");
  await Bun.write(
    input,
    name === "unlit"
      ? `export const composition={width:${width},height:${height},fps:30,duration:2};
const BALL=lathe(path("M 0 -1 C 0.5523 -1 1 -0.5523 1 0 C 1 0.5523 0.5523 1 0 1"),{segments:8});
export default function Main(){const time=useCurrentTime();return <Scene>
<Scene3D key="gpu" style={{width:${width},height:${height}}} camera={{position:[0,0,5],target:[0,0,0],fov:45}}>
<Mesh key="ball" geometry={BALL} rotateY={time.seconds*90} material={{type:"unlit",color:"#cf7338"}}/>
</Scene3D></Scene>;}`
      : name === "clip"
        ? `export const composition={width:${width},height:${height},fps:30,duration:2};
export const controls={assets:{model:asset({kind:"model3d"})}};
const textures={baseColor:null,metallicRoughness:null,normal:null,occlusion:null,emissive:null};
export default function Main(){const time=useCurrentTime();return <Scene style={{backgroundColor:"#102030"}}>
<Scene3D key="gpu" style={{width:${width},height:${height}}} camera={{position:[0,0,3],target:[0,0,0],fov:45,near:time.seconds<1?0.1:3,far:time.seconds<1?3:100}}>
<Mesh key="first" src="asset://model" material={{type:"unlit",color:time.seconds<1?"#cc201066":"#cc2010ff",alphaMode:"mask",alphaCutoff:0.5,emissiveIntensity:0,textures}}/>
<Mesh key="second" src="asset://model" material={{type:"unlit",color:"#1020cc",alphaMode:"mask",alphaCutoff:0.5,emissiveIntensity:0,textures}}/>
</Scene3D></Scene>;}`
        : animated
          ? `export const composition={width:${width},height:${height},fps:30,duration:2};
export const controls={assets:{model:asset({kind:"model3d"})}};
export default function Main(){const time=useCurrentTime();return <Scene style={{backgroundColor:"#102030"}}>
<Scene3D key="gpu" style={{width:${width},height:${height}}} camera={{position:[0,0,${name === "skin" ? 15 : 5}],target:[0,0,0],fov:45}}>
<Mesh key="model" src="asset://model" animation={{clip:0,time:time.seconds*2}} rotateY={time.seconds*45} scale={[-1,1,1]} material={{type:"pbr",color:"#cf7338",roughness:0.4}}/>
<AmbientLight intensity={0.4}/><DirectionalLight direction={[0.4,0.8,1]} intensity={1.2}/>
</Scene3D></Scene>;}`
          : source(resources),
  );
  const child = Bun.spawn(
    [
      cli,
      "--json",
      "motion",
      "studio",
      input,
      "--port",
      "0",
      "--web-assets-dir",
      runtime,
      ...(animated || name === "clip"
        ? [
            "--asset",
            `model=${join(root, `crates/valle-motion/tests/fixtures/scene3d/${name === "clip" ? "pbr" : name === "skin" ? "rigged-simple" : "animated-morph-cube"}.glb`)}`,
          ]
        : []),
      ...(resources
        ? [
            "--asset",
            `map=${join(evidence, "map.png")}`,
            "--asset",
            `sky=${join(evidence, "map.png")}`,
            "--asset",
            `model=${join(root, "crates/valle-motion/tests/fixtures/scene3d/triangle.glb")}`,
          ]
        : []),
    ],
    { stdout: "pipe", stderr: Bun.file(join(directory, "studio.log")) },
  );
  let chrome: Bun.Subprocess | undefined, host: ReturnType<typeof Bun.serve> | undefined;
  try {
    const reader = child.stdout.getReader();
    let line = "";
    try {
      while (!line.includes("\n")) {
        const part = await deadline(reader.read());
        if (part.done)
          throw new Error(`Studio exited: ${await Bun.file(join(directory, "studio.log")).text()}`);
        line += new TextDecoder().decode(part.value);
      }
    } finally {
      reader.releaseLock();
    }
    const upstream = JSON.parse(line.split("\n")[0]!).url as string;
    let resolveReport: (report: Record<string, unknown>) => void = () => {};
    const result = new Promise<Record<string, unknown>>((resolve) => {
      resolveReport = resolve;
    });
    const stages: unknown[] = [];
    const started = Date.now();
    let lastActivity = started;
    const preview = join(root, "web/test-dist/preview");
    host = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      async fetch(request) {
        const url = new URL(request.url);
        if (url.pathname === "/result") {
          resolveReport(await request.json());
          return new Response("ok");
        }
        if (url.pathname === "/progress") {
          lastActivity = Date.now();
          const stage = { elapsedMs: Date.now() - started, ...(await request.json()) };
          stages.push(stage);
          await Bun.write(join(directory, "progress.json"), JSON.stringify(stages, null, 2));
          console.error(`Scene3D ${name}: ${JSON.stringify(stage)}`);
          return new Response("ok");
        }
        if (
          ["/scene3d.mp4", "/scene3d-cpu.png", "/scene3d-gpu.png"].includes(url.pathname) &&
          request.method === "POST"
        ) {
          await Bun.write(join(directory, url.pathname.slice(1)), await request.arrayBuffer());
          return new Response("ok");
        }
        const local = resolve(
          preview,
          `.${url.pathname === "/preview.html" ? "/index.html" : url.pathname}`,
        );
        if (local.startsWith(`${preview}/`) && (await Bun.file(local).exists()))
          return new Response(Bun.file(local));
        return fetch(new URL(url.pathname + url.search, upstream));
      },
    });
    const pageUrl = `${host.url}preview.html?scene3dprobe=1&features=${name}${width < 3840 && name !== "unlit" ? "&encode=1" : ""}`,
      stderrPath = join(directory, "chrome.log");
    chrome = Bun.spawn(
      [
        ...browserLaunchCommand(browser),
        "--headless=new",
        "--no-sandbox",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-extensions",
        ...(process.argv.includes("--swiftshader")
          ? ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"]
          : []),
        `--user-data-dir=${join(directory, "chrome")}`,
        pageUrl,
      ],
      { stdout: "ignore", stderr: Bun.file(stderrPath) },
    );
    let completed = false;
    const exit = waitForUnexpectedBrowserExit(chrome, {
      operation: "Scene3D browser acceptance",
      binary: browser,
      pageUrl,
      stderrPath,
      isCompleted: () => completed,
    });
    // Software GPU drivers compile and encode on the CI CPU. Bound both stalls
    // and total time without charging all advancing stages one two-minute budget.
    const report = await deadline(Promise.race([result, exit]), () => lastActivity);
    completed = true;
    await Bun.write(join(directory, "report.json"), JSON.stringify(report, null, 2));
    if (report.status !== "ok") throw new Error(JSON.stringify(report));
    console.log(JSON.stringify({ name, ...report }));
  } finally {
    if (chrome) await terminateBrowser(chrome);
    host?.stop(true);
    child.kill();
    await child.exited;
    console.error(`Scene3D evidence: ${directory}`);
  }
}
