import { appendFile, mkdir, readdir } from "node:fs/promises";
import path from "node:path";

// Pinned shared builds for the supported FFmpeg APIs. Month-end builds are retained for two
// years; keep each tag, build, archive suffix and pair of hashes together. Normal CI uses 9.
const releases: Record<string, { tag: string; build: string; suffix: string; hashes: string[] }> = {
  "7": {
    tag: "autobuild-2025-12-31-14-28",
    build: "n7.1.3-22-g40b336e650",
    suffix: "7.1",
    hashes: [
      "91c0307f32c8d782751893edd6a717d3d8b7c2fc179ccd0a8ae14077faa73108",
      "de4b3b27d2f27a4c923ec8d982dda935edb78542528c4599008f3053a71958bd",
    ],
  },
  "8": {
    tag: "autobuild-2026-09-30-13-08",
    build: "n8.1.3-9-g29e619e767",
    suffix: "8.1",
    hashes: [
      "76649c19a7a4615b367251ba4a8adc643f65831278a00e7a0c38398f7f06484b",
      "dfe81c3aa0a546ee81980b1b824486dd0fe0a1d452f00cb55135379a74be7bcf",
    ],
  },
  "9": {
    tag: "autobuild-2026-09-30-13-08",
    build: "n9.0.2-17-g2a571b6068",
    suffix: "9.0",
    hashes: [
      "01a9764d0b5364b66cfeb4617557c64321b0e232e172ec73f0a701c5f7694326",
      "3da6c7b60bb9ccd73ec5b5e815ba804a0879eb362ba0e3beebce50174c022696",
    ],
  },
};
const major = process.env.VALLE_TEST_FFMPEG_MAJOR ?? "9";
const selected = releases[major];
if (!selected) throw new Error(`Unsupported FFmpeg major: ${major}`);
function required(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required; this installer is for GitHub Actions`);
  return value;
}
// Build tools may run under x64 emulation on Windows Arm; DLLs must match the native Rust target.
const arch = ({ X64: "x64", ARM64: "arm64" } as Record<string, string>)[required("RUNNER_ARCH")];
if (!["linux", "win32"].includes(process.platform) || arch !== "x64")
  throw new Error(`Unsupported CI FFmpeg host: ${process.platform}-${process.env.RUNNER_ARCH}`);
const root = path.join(required("RUNNER_TEMP"), `valle-ffmpeg-${major}`);
const windows = process.platform === "win32";
const target = windows ? "win64" : "linux64";
const sha256 = selected.hashes[windows ? 1 : 0];
const directory = `ffmpeg-${selected.build}-${target}-gpl-shared-${selected.suffix}`;
const archive = `${directory}.${windows ? "zip" : "tar.xz"}`;
const url = `https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases/download/${selected.tag}/${archive}`;
await mkdir(root, { recursive: true });
let bytes: ArrayBuffer | undefined;
for (let attempt = 1; attempt <= 3; attempt++) {
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(180_000) });
    if (!response.ok) throw new Error(`HTTP ${response.status}: ${url}`);
    bytes = await response.arrayBuffer();
    const actual = new Bun.CryptoHasher("sha256").update(bytes).digest("hex");
    if (actual !== sha256) throw new Error(`SHA-256 mismatch for ${archive}`);
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
const extract = Bun.spawn([tar, "-xf", archivePath, "-C", root], {
  stdout: "inherit",
  stderr: "inherit",
});
if ((await extract.exited) !== 0) throw new Error("FFmpeg archive extraction failed");
const bin = path.join(root, directory, "bin");
const libraries = windows ? bin : path.join(root, directory, "lib");
const files = await readdir(libraries);
for (const name of ["avcodec", "avformat", "avutil", "swscale", "swresample"]) {
  if (
    !files.some(
      (file) =>
        file.startsWith(windows ? name : `lib${name}`) &&
        (windows ? file.endsWith(".dll") : /\.so(?:\.|$)/.test(file)),
    )
  ) {
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
    env: { ...process.env, LD_LIBRARY_PATH: libraries },
    stdout: "inherit",
    stderr: "inherit",
  });
  const exitCode = await version.exited;
  if (exitCode !== 0) throw new Error(`FFmpeg test runtime failed to start: exit ${exitCode}`);
}
