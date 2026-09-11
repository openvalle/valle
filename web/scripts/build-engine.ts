import { mkdir, rm } from "node:fs/promises";
import path from "node:path";
import { withBuildDirectory } from "./build-directory.ts";

const root = path.resolve(import.meta.dir, "..");
const workspace = path.resolve(root, "..");
const outDir = path.resolve(root, "packages", "engine", "generated", "web");

async function run(args: string[], env = process.env): Promise<void> {
  const child = Bun.spawn(args, { cwd: workspace, env, stdout: "inherit", stderr: "inherit" });
  if (await child.exited !== 0) throw new Error(`engine build failed: ${args.join(" ")}`);
}

const metadataProcess = Bun.spawn(["cargo", "metadata", "--no-deps", "--locked", "--format-version", "1"], {
  cwd: workspace, stdout: "pipe", stderr: "inherit",
});
const metadataText = await new Response(metadataProcess.stdout).text();
if (await metadataProcess.exited !== 0) throw new Error("cannot read Cargo workspace metadata");
const metadata = JSON.parse(metadataText) as {
  target_directory: string;
  packages: Array<{ name: string; version: string; description: string; license: string; repository: string }>;
};
const engine = metadata.packages.find(pkg => pkg.name === "valle-engine");
if (!engine) throw new Error("valle-engine is missing from the Cargo workspace");

const lock = Bun.TOML.parse(await Bun.file(path.join(workspace, "Cargo.lock")).text()) as {
  package: Array<{ name: string; version: string }>;
};
const versions = new Set(lock.package.filter(pkg => pkg.name === "wasm-bindgen").map(pkg => pkg.version));
if (versions.size !== 1) throw new Error("expected exactly one wasm-bindgen version in Cargo.lock");
const version = [...versions][0]!;

async function matchesVersion(binary: string): Promise<boolean> {
  if (!await Bun.file(binary).exists()) return false;
  const child = Bun.spawn([binary, "--version"], { stdout: "pipe", stderr: "pipe" });
  const output = await new Response(child.stdout).text();
  return await child.exited === 0 && output.trim() === `wasm-bindgen ${version}`;
}

// Cache the exact CLI version beside build artifacts; never replace a user's global tool.
const installRoot = path.join(metadata.target_directory, "tools", `wasm-bindgen-${version}`);
const cached = path.join(installRoot, "bin", process.platform === "win32" ? "wasm-bindgen.exe" : "wasm-bindgen");
const fromPath = Bun.which("wasm-bindgen");
let bindgen: string;
if (await matchesVersion(cached)) bindgen = cached;
else if (fromPath && await matchesVersion(fromPath)) bindgen = fromPath;
else {
  const targets: Record<string, string> = {
    "darwin-arm64": "aarch64-apple-darwin", "darwin-x64": "x86_64-apple-darwin",
    "linux-arm64": "aarch64-unknown-linux-musl", "linux-x64": "x86_64-unknown-linux-musl",
    "win32-x64": "x86_64-pc-windows-msvc",
  };
  const target = targets[`${process.platform}-${process.arch}`];
  if (!target) throw new Error(`Install wasm-bindgen-cli ${version} on PATH for ${process.platform}-${process.arch}`);
  // Git's GNU tar treats a Windows drive letter as a remote archive host.
  const systemRoot = process.env.SystemRoot ?? process.env.WINDIR;
  const tar = process.platform === "win32"
    ? systemRoot && path.join(systemRoot, "System32", "tar.exe")
    : Bun.which("tar");
  if (!tar) throw new Error("tar is required to extract the wasm-bindgen release");
  console.log(`Downloading wasm-bindgen-cli ${version} for ${target}`);
  const release = `wasm-bindgen-${version}-${target}`;
  const url = `https://github.com/wasm-bindgen/wasm-bindgen/releases/download/${version}/${release}.tar.gz`;
  async function download(url: string): Promise<Response> {
    const response = await fetch(url, { signal: AbortSignal.timeout(120_000) });
    if (!response.ok) throw new Error(`Download failed (${response.status}): ${url}`);
    return response;
  }
  await withBuildDirectory(installRoot, async (next) => {
    const checksum = (await (await download(`${url}.sha256sum`)).text()).trim().split(/\s+/)[0]!;
    const archive = await (await download(url)).arrayBuffer();
    const actual = new Bun.CryptoHasher("sha256").update(archive).digest("hex");
    if (!/^[a-f0-9]{64}$/.test(checksum) || actual !== checksum) {
      throw new Error(`SHA-256 mismatch for ${release}`);
    }
    const archivePath = path.join(next, "release.tar.gz");
    const bin = path.join(next, "bin");
    const binary = path.basename(cached);
    await Bun.write(archivePath, archive);
    await mkdir(bin);
    await run([tar, "-xzf", archivePath, "-C", bin, "--strip-components=1", `${release}/${binary}`]);
    if (!await matchesVersion(path.join(bin, binary))) throw new Error(`downloaded wasm-bindgen does not match ${version}`);
    await rm(archivePath);
  });
  bindgen = cached;
}

await withBuildDirectory(outDir, async (next) => {
  await run(["cargo", "build", "--release", "--locked", "-p", "valle-engine", "--target", "wasm32-unknown-unknown", "--features", "web"]);
  await run([bindgen, path.join(metadata.target_directory, "wasm32-unknown-unknown", "release", "valle_engine.wasm"),
    "--target", "web", "--out-dir", next, "--out-name", "valle_engine"]);
  await Bun.write(path.join(next, "LICENSE"), Bun.file(path.join(workspace, "LICENSE")));
  await Bun.write(path.join(next, "package.json"), JSON.stringify({
    name: engine.name, version: engine.version, private: true, description: engine.description,
    license: engine.license, repository: { type: "git", url: engine.repository },
    files: ["*.wasm", "*.js", "*.d.ts", "snippets", "LICENSE"],
    module: "valle_engine.js", types: "valle_engine.d.ts", sideEffects: ["./snippets/*"],
  }, null, 2) + "\n");
});
