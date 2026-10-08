import { appendFile, mkdir, readdir } from "node:fs/promises";
import path from "node:path";

// Month-end BtbN builds are retained for two years. Keep the tag, build and hashes together.
const release = "autobuild-2026-09-30-13-08";
const build = "n9.0.2-17-g2a571b6068";
const platforms: Record<string, { target: string; sha256: string; release?: string; build?: string }> = {
  "linux-x64": { target: "linux64", sha256: "01a9764d0b5364b66cfeb4617557c64321b0e232e172ec73f0a701c5f7694326" },
  "linux-arm64": { target: "linuxarm64", sha256: "706ccdbc8b0537646345532713a9b6750c0a6258cc001ec1c0379754f0ae730a" },
  "win32-x64": { target: "win64", sha256: "3da6c7b60bb9ccd73ec5b5e815ba804a0879eb362ba0e3beebce50174c022696" },
  // September's ARM64 binaries were corrupted by LLVM strip (BtbN/FFmpeg-Builds#675).
  "win32-arm64": {
    target: "winarm64", release: "autobuild-2026-08-31-13-27", build: "n9.0.1-11-ge47273f4d9",
    sha256: "4419887ac2c5585d909d532944843756f67e430d4f3822e9062bff1455498abc",
  },
};
function required(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required; this installer is for GitHub Actions`);
  return value;
}
// Build tools may run under x64 emulation on Windows Arm; DLLs must match the native Rust target.
const arch = ({ X64: "x64", ARM64: "arm64" } as Record<string, string>)[required("RUNNER_ARCH")];
const selected = platforms[`${process.platform}-${arch}`];
if (!selected) throw new Error(`Unsupported CI FFmpeg host: ${process.platform}-${process.env.RUNNER_ARCH}`);
const root = path.join(required("RUNNER_TEMP"), "valle-ffmpeg");
const windows = process.platform === "win32";
const directory = `ffmpeg-${selected.build ?? build}-${selected.target}-gpl-shared-9.0`;
const archive = `${directory}.${windows ? "zip" : "tar.xz"}`;
const url = `https://github.com/BtbN/FFmpeg-Builds/releases/download/${selected.release ?? release}/${archive}`;
await mkdir(root, { recursive: true });
let bytes: ArrayBuffer | undefined;
for (let attempt = 1; attempt <= 3; attempt++) {
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(180_000) });
    if (!response.ok) throw new Error(`HTTP ${response.status}: ${url}`);
    bytes = await response.arrayBuffer();
    const actual = new Bun.CryptoHasher("sha256").update(bytes).digest("hex");
    if (actual !== selected.sha256) throw new Error(`SHA-256 mismatch for ${archive}`);
    break;
  } catch (error) {
    if (attempt === 3) throw error;
    console.warn(`FFmpeg download attempt ${attempt} failed: ${error}`);
    await Bun.sleep(attempt * 1000);
  }
}
const archivePath = path.join(root, archive);
await Bun.write(archivePath, bytes!);
const tar = windows ? path.join(required("SystemRoot"), "System32", "tar.exe") : "tar";
const extract = Bun.spawn([tar, "-xf", archivePath, "-C", root], { stdout: "inherit", stderr: "inherit" });
if (await extract.exited !== 0) throw new Error("FFmpeg archive extraction failed");
const bin = path.join(root, directory, "bin");
const libraries = windows ? bin : path.join(root, directory, "lib");
const files = await readdir(libraries);
for (const name of ["avcodec", "avformat", "avutil", "swscale", "swresample"]) {
  if (!files.some(file => file.startsWith(windows ? name : `lib${name}`)
    && (windows ? file.endsWith(".dll") : /\.so(?:\.|$)/.test(file)))) {
    throw new Error(`Missing FFmpeg ${name} shared library in ${libraries}`);
  }
}
await appendFile(required("GITHUB_ENV"), `VALLE_FFMPEG_DIR=${libraries}\n`);
await appendFile(required("GITHUB_PATH"), `${bin}\n`);
if (!windows) {
  await appendFile(required("GITHUB_ENV"), `LD_LIBRARY_PATH=${libraries}\n`);
}
// Windows validates with PowerShell so native NTSTATUS values are retained.
if (!windows) {
  const version = Bun.spawn([path.join(bin, "ffmpeg"), "-version"], {
    env: { ...process.env, LD_LIBRARY_PATH: libraries }, stdout: "inherit", stderr: "inherit",
  });
  const exitCode = await version.exited;
  if (exitCode !== 0) throw new Error(`FFmpeg test runtime failed to start: exit ${exitCode}`);
}
