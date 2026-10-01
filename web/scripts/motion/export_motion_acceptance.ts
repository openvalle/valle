/** Export the 36 Chapter 8 scenes and a local video gallery with the release CLI.
 * Run on a macOS Metal host: bun web/scripts/motion/export_motion_acceptance.ts
 * Every run regenerates evidence from the current sources, assets and release CLI.
 */
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { ROOT, run, runJson } from "./common.ts";
import { writeOverview, writeShowcaseTexture } from "./acceptance_images.ts";
import { sphereGlb } from "./generate_sphere_glb.ts";

const CLI = join(ROOT, "target/release/valle");
const FIXTURES = join(ROOT, "crates/valle-compiler/tests/fixtures/motion/composition");
const OUT = join(ROOT, "target/motion-acceptance-videos");
const VIDEOS = join(OUT, "videos");
const EVIDENCE = join(OUT, "evidence");
const SOURCES = join(OUT, "sources");
const THUMBS = join(OUT, "thumbnails");

type Case = readonly [id: string, title: string, stem: string | null];
const CASES: Case[] = [
  ["A01", "自动运动模糊", "auto-motion-blur"],
  ["A02", "快门采样", "shutter-sampling"],
  ["A03", "回声", "echo-trails"],
  ["A04", "时间作用域", "time-scope"],
  ["A05", "缓动预设", "easing-presets"],
  ["A06", "粒子力场", "particle-forces"],
  ["A07", "烘焙模拟", "spring-simulation"],
  ["B01", "自动实例化 · 4096 元素", null],
  ["B02", "批量路径", "path-geometry-batch"],
  ["B03", "按需求值", "lazy-evaluation"],
  ["C01", "文字描绘", "text-outline-drawing"],
  ["C02", "选区与逐字模糊", "text-range-selector"],
  ["C03", "富文本逐字动画", "rich-text-units"],
  ["C04", "平滑字重", "variable-font-weight"],
  ["C05", "多行路径文字", "multiline-path-text"],
  ["C06", "3D 挤出字", "extruded-text"],
  ["D01", "星形到圆形", "star-to-circle-morph"],
  ["D02", "形变序列", "path-morph-sequence"],
  ["D03", "路径修饰器", "path-modifiers"],
  ["D04", "反向拖尾", "reverse-path-trail"],
  ["E01", "线性光加色", "additive-light"],
  ["E02", "子树遮罩转场", "subtree-mask"],
  ["E03", "线性光 Screen", "linear-light-blend"],
  ["E04", "OKLCH 颜色插值", "oklch-color-interpolation"],
  ["E05", "OKLab CSS 渐变", "css-oklab"],
  ["E06", "Motion 转场", "motion-transition"],
  ["F01", "发光", "node-glow"],
  ["F02", "场景 Bloom", "scene-bloom"],
  ["F03", "色差", "chromatic-aberration"],
  ["G01", "音频驱动", null],
  ["H01", "ProRes 4444 透明 MOV", "transparent-video-scene"],
  ["H02", "多帧与联系表", "multi-frame-motion-scene"],
  ["H03", "频闪审片", "strobe-review"],
  ["circle-open-transition", "Timeline 圆形转场", null],
  ["H05", "裸文件名模块解析", null],
  ["I01", "1080p 球体 Scene3D · 光照演示", "sphere-scene3d"],
];

const G01_SOURCE = `export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export const controls = { assets: { beat: asset({ kind: "audio", required: true }) } };
const BEAT = audioAnalysis("asset://beat", { bands: 4, fps: 30 });
export default function AudioDriven(ctx) {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    <View key="bar" className="absolute" style={{ left: 300, bottom: 20, width: 40,
      height: 10 + BEAT.level(ctx.seconds) * 250, backgroundColor: "#ffffff" }} />
  </Scene>;
}
`;

const H05_SOURCE = `import { SIZE } from "./lib";
export const composition = { width: 640, height: 360, fps: 30, duration: 1 };
export default function Main(ctx) {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    <View key="box" className="absolute" style={{ left: 20, top: 20, width: SIZE,
      height: SIZE, backgroundColor: "#ffffff" }} />
  </Scene>;
}
`;

const I01_SOURCE = `export const composition = { width: 1920, height: 1080, fps: 30, duration: 2 };
export const controls = { assets: {
  model: asset({ kind: "model3d", required: true }),
  surface: asset({ kind: "image", required: true }),
} };
export default function LitSphere(ctx) {
  return <Scene style={{ width: 1920, height: 1080, backgroundColor: "#07101d" }}>
    <Scene3D key="stage" style={{ width: 1920, height: 1080 }}
      camera={{ position: [0, 0, 3.2], target: [0, 0, 0], fov: 45 }}
      pbr={{ toneMapping: "aces", exposure: 1 }}>
      <AmbientLight intensity={0.25} />
      <DirectionalLight color="#fff1ce" direction={[0.7, 0.8, 1]} intensity={2.8} />
      <DirectionalLight color="#69bfff" direction={[-1, -0.2, 0.2]} intensity={0.7} />
      <Mesh key="sphere" src="asset://model" rotation={[12, ctx.seconds * 120, 0]}
        material={{ type: "pbr", color: "#ffffff", metallic: 0.05, roughness: 0.45,
          textures: { baseColor: "asset://surface" } }} />
    </Scene3D>
  </Scene>;
}
`;

function writeWav(path: string) {
  const frames = 96_000;
  const buffer = new Uint8Array(44 + frames * 2);
  const view = new DataView(buffer.buffer);
  buffer.set(new TextEncoder().encode("RIFF"), 0); view.setUint32(4, buffer.length - 8, true);
  buffer.set(new TextEncoder().encode("WAVEfmt "), 8); view.setUint32(16, 16, true);
  view.setUint16(20, 1, true); view.setUint16(22, 1, true);
  view.setUint32(24, 48_000, true); view.setUint32(28, 96_000, true);
  view.setUint16(32, 2, true); view.setUint16(34, 16, true);
  buffer.set(new TextEncoder().encode("data"), 36); view.setUint32(40, frames * 2, true);
  for (let index = 0; index < frames; index++) {
    const amplitude = index < 48_000 ? 0 : Math.trunc(Math.sin(2 * Math.PI * 440 * (index % 48_000) / 48_000) * 24_000);
    view.setInt16(44 + index * 2, amplitude, true);
  }
  writeFileSync(path, buffer);
}

function sourceFor(id: string, stem: string | null): string {
  if (id === "B01") {
    const source = join(SOURCES, "grid-4096-instances.motion.tsx");
    const original = readFileSync(join(FIXTURES, "grid-instances.motion.tsx"), "utf8");
    if (!original.includes("const COLS = 8;")) throw new Error("B01 fixture changed its column declaration");
    writeFileSync(source, original.replace("const COLS = 8;", "const COLS = 64;"));
    return source;
  }
  if (id === "G01") {
    const source = join(SOURCES, "audio-driven-level.motion.tsx");
    writeFileSync(source, G01_SOURCE);
    writeWav(join(SOURCES, "G01-beat.wav"));
    return source;
  }
  if (id === "circle-open-transition") return join(FIXTURES, "circle-open-transition.timeline.json");
  if (id === "H05") {
    const directory = join(SOURCES, "module-import");
    mkdirSync(directory, { recursive: true });
    writeFileSync(join(directory, "lib.motion.ts"), "export const SIZE = 64;\n");
    const source = join(directory, "module-entry.motion.tsx");
    writeFileSync(source, H05_SOURCE);
    return source;
  }
  if (id === "I01") {
    const source = join(SOURCES, "lit-sphere-scene3d.motion.tsx");
    writeFileSync(source, I01_SOURCE);
    const sphere = join(SOURCES, "I01-uv-sphere.glb");
    writeFileSync(sphere, sphereGlb({
      segments: 64, rings: 32, withUvs: true, generator: "Valle I01 lit showcase",
    }));
    const texture = join(SOURCES, "I01-surface.png");
    writeShowcaseTexture(texture);
    return source;
  }
  if (!stem) throw new Error(`missing fixture for ${id}`);
  return join(FIXTURES, `${stem}.motion.tsx`);
}

function saveReport(id: string, name: string, report: unknown) {
  writeFileSync(join(EVIDENCE, `${id}-${name}.json`), JSON.stringify(report, null, 2) + "\n");
}

type Delivery = { backend: string; timing: { renderMs: number } };
type RenderReport = { status: string; delivery: Delivery; compilations?: number };
type Probe = { streams: Array<{ codec_name: string; width?: number; height?: number; nb_frames?: string }> };

function render(id: string, source: string, output: string, extra: string[] = []): RenderReport {
  const extension = output.split(".").at(-1)!;
  const reportPath = join(EVIDENCE, `${id}-${extension}-render.json`);
  // Existence does not establish provenance (including imported modules/assets).
  // Regenerate acceptance evidence instead of silently accepting a stale run.
  rmSync(output, { force: true });
  rmSync(reportPath, { force: true });
  const argv = id === "circle-open-transition"
    ? [CLI, "timeline", "render", source, "-o", output, "--json"]
    : [CLI, "motion", "render", source, "-o", output, "--backend", "metal", "--json"];
  const report = runJson<RenderReport>([...argv, ...extra]);
  if (report.status !== "ok" || (id !== "circle-open-transition" && report.delivery.backend !== "metal")) {
    throw new Error(`${id}: render did not use the expected backend`);
  }
  saveReport(id, `${extension}-render`, report);
  return report;
}

function probe(path: string): Probe {
  return runJson<Probe>(["ffprobe", "-v", "error", "-show_entries",
    "stream=codec_name,pix_fmt,width,height,nb_frames:format=duration", "-of", "json", path]);
}

function thumbnail(id: string, video: string) {
  const output = join(THUMBS, `${id}.jpg`);
  rmSync(output, { force: true });
  const second = id === "circle-open-transition" ? "2" : ["H01", "H05", "I01"].includes(id) ? "0.5" : "1";
  run(["ffmpeg", "-v", "error", "-ss", second, "-i", video,
    "-frames:v", "1", "-vf", "scale=320:-2", "-q:v", "3", output]);
}

type Row = { id: string; title: string; source: string; video: string;
  backend: string; codec: string; width: number; height: number; frames: number };
function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, char => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;",
    '"': "&quot;", "'": "&#39;" })[char]!);
}

function gallery(rows: Row[]) {
  const cards = rows.map(row => {
    const { id } = row;
    const revision = id === "I01" ? "?v=lit-sphere" : id === "F01" ? "?v=f01-metal-edge" : "";
    const extras = id === "H01" ? '<a href="videos/H01.mov">下载 ProRes 4444 透明 MOV</a>'
      : id === "H02" ? '<a href="evidence/H02-sheet.png">查看 0/30/59 帧联系表</a>'
      : id === "H03" ? '<a href="evidence/H03-review.json">查看审片报告</a>'
      : id === "H05" ? '<a href="evidence/H05-check.json">查看裸文件名检查结果</a>'
      : id === "I01" && existsSync(join(VIDEOS, "I01-contract.mp4"))
        ? '<a href="videos/I01-contract.mp4">查看无光照的原始验收视频</a>' : "";
    return `<article id="${id}"><h2>${id} <span>${escapeHtml(row.title)}</span></h2>
<video controls loop preload="metadata" poster="thumbnails/${id}.jpg${revision}" src="videos/${id}.mp4${revision}"></video>
<p><a href="videos/${id}.mp4">下载视频</a> ${extras}</p></article>`;
  });
  const page = `<!doctype html><html lang="zh-CN"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Motion 第 8 章 · 36 个验收例子</title>
<style>body{margin:0;background:#0e1118;color:#edf2ff;font:16px system-ui,sans-serif}
header{padding:32px 4vw 20px}h1{margin:0 0 8px;font-size:32px}header p{color:#a6b1c6}
nav{display:flex;flex-wrap:wrap;gap:8px;margin-top:20px}nav a{padding:6px 10px;background:#202b3e;border-radius:6px}
a{color:#9fc6ff;text-decoration:none}a:hover{text-decoration:underline}
main{padding:12px 4vw 50px;display:grid;grid-template-columns:repeat(auto-fit,minmax(320px,1fr));gap:20px}
article{background:#171e2b;border:1px solid #2a3547;border-radius:12px;padding:14px}
h2{font-size:17px;margin:0 0 10px}h2 span{font-weight:400;color:#a6b1c6}
video{display:block;width:100%;aspect-ratio:16/9;background:#000;border-radius:6px}
article p{display:flex;gap:12px;flex-wrap:wrap;font-size:13px;margin:10px 0 0}</style>
<header><h1>Motion 第 8 章 · 36 个验收例子</h1>
<p>35 个 Motion 视频在真实 macOS Metal 上导出；Timeline 圆形转场由 Timeline CLI 的 Raster 后端导出。H01 另附透明 ProRes 4444 MOV。视频用于看画面，详细判定仍以各用例的测试和报告为准。</p>
<p><a href="overview.jpg">查看 36 格总览</a> · <a href="all-videos.zip">下载全部视频及验收产物</a></p>
<nav>${rows.map(row => `<a href="#${row.id}">${row.id}</a>`).join("")}</nav></header>
<main>${cards.join("\n")}</main></html>\n`;
  writeFileSync(join(OUT, "index.html"), page);
}

function archive(rows: Row[]) {
  const files = rows.map(row => row.video).concat([
    "videos/H01.mov", "evidence/H02-sheet.png", "evidence/H02-sheet.json",
    "evidence/H03-review.json", "evidence/H05-check.json", "overview.jpg",
    "index.html", "manifest.json",
  ]);
  if (existsSync(join(VIDEOS, "I01-contract.mp4"))) files.push("videos/I01-contract.mp4");
  const output = join(OUT, "all-videos.zip");
  rmSync(output, { force: true });
  run(["zip", "-q", "-9", output, ...files], { cwd: OUT });
}

function main() {
  for (const path of [VIDEOS, EVIDENCE, SOURCES, THUMBS]) mkdirSync(path, { recursive: true });
  if (!existsSync(CLI)) throw new Error(`build release CLI first: ${CLI}`);
  for (const name of ["index.html", "overview.jpg", "all-videos.zip"]) rmSync(join(OUT, name), { force: true });
  const rows: Row[] = [];
  const errors: Record<string, string> = {};
  for (const [id, title, stem] of CASES) {
    console.log(`${id} ${title}`);
    try {
      const source = sourceFor(id, stem);
      const extra = id === "G01" ? ["--asset", `beat=${join(SOURCES, "G01-beat.wav")}`]
        : id === "I01" ? ["--asset", `model=${join(SOURCES, "I01-uv-sphere.glb")}`,
          "--asset", `surface=${join(SOURCES, "I01-surface.png")}`] : [];
      if (id === "H05") {
        saveReport(id, "check", runJson<unknown>([CLI, "motion", "check", "module-entry.motion.tsx", "--json"],
          { cwd: join(SOURCES, "module-import") }));
      }
      if (id === "H02") {
        rmSync(join(EVIDENCE, "H02-sheet.png"), { force: true });
        const checked = runJson<{ compilations: number }>([CLI, "motion", "render", source,
          "--backend", "metal", "--frames", "0,30,59", "--storyboard",
          join(EVIDENCE, "H02-sheet.png"), "--json"]);
        if (checked.compilations !== 1) throw new Error("H02 rendered with multiple compilations");
        saveReport(id, "sheet", checked);
      }
      if (id === "H03") {
        saveReport(id, "review", runJson<unknown>([CLI, "motion", "review", source, "--json", "--trajectories"]));
      }
      if (id === "H01") render(id, source, join(VIDEOS, "H01.mov"), ["--codec", "prores4444"]);
      let video: string, report: RenderReport;
      if (id === "G01") {
        const visual = join(VIDEOS, "G01-visual.mp4");
        report = render(id, source, visual, extra);
        video = join(VIDEOS, "G01.mp4");
        rmSync(video, { force: true });
        run(["ffmpeg", "-v", "error", "-i", visual,
          "-i", join(SOURCES, "G01-beat.wav"), "-c:v", "copy", "-c:a", "aac",
          "-b:a", "160k", "-shortest", video]);
      } else {
        video = join(VIDEOS, `${id}.mp4`);
        report = render(id, source, video, extra);
      }
      const info = probe(video);
      const stream = info.streams.find(entry => entry.width);
      if (!stream?.width || !stream.height || !stream.nb_frames) throw new Error(`${id}: missing video stream`);
      if (id !== "circle-open-transition" && report.delivery.backend !== "metal") throw new Error(`${id}: expected Metal output`);
      if (id === "H01") {
        const mov = probe(join(VIDEOS, "H01.mov"));
        if (mov.streams[0]?.codec_name !== "prores") throw new Error("H01 is not ProRes");
        saveReport(id, "mov-probe", mov);
      }
      saveReport(id, "mp4-probe", info);
      thumbnail(id, video);
      rows.push({ id, title, source: relative(ROOT, source), video: relative(OUT, video),
        backend: id === "circle-open-transition" ? "timeline auto" : report.delivery.backend,
        codec: stream.codec_name, width: stream.width, height: stream.height,
        frames: Number(stream.nb_frames) });
    } catch (error) {
      errors[id] = String(error);
      console.error(`  FAILED: ${errors[id]}`);
    }
  }
  writeFileSync(join(OUT, "manifest.json"), JSON.stringify({ cases: rows, errors }, null, 2) + "\n");
  if (rows.length === CASES.length) {
    gallery(rows);
    writeOverview(rows.map(row => row.id), THUMBS, join(OUT, "overview.jpg"));
    archive(rows);
  }
  console.log(`Exported ${rows.length}/${CASES.length}, errors: ${Object.keys(errors)}; ${OUT}`);
  if (Object.keys(errors).length) process.exitCode = 1;
}

if (import.meta.main) main();
