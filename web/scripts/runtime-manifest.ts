import { mkdir, readdir } from "node:fs/promises";
import path from "node:path";

export const RUNTIME_MANIFEST_SCHEMA_VERSION = 2 as const;

export interface RuntimeAsset {
  id: string;
  owner: string;
  role: string;
  path: string;
  bytes: number;
  sha256: string;
  license: string;
  group?: string;
  source?: string;
}

export interface PackageRuntimeManifest {
  schemaVersion: typeof RUNTIME_MANIFEST_SCHEMA_VERSION;
  package: string;
  assets: RuntimeAsset[];
}

export interface RootRuntimeManifest {
  schemaVersion: typeof RUNTIME_MANIFEST_SCHEMA_VERSION;
  runtimeVersion: string;
  protocolVersion: number;
  packages: Array<{ id: string; manifest: string }>;
  assets: RuntimeAsset[];
  assetGroups: Array<{ id: string; glue: string; wasm: string[] }>;
  workers: Array<{ id: string; path: string }>;
  apps: Array<{ id: string; html: string }>;
  routes: Record<string, string>;
  runtimeAssets: {
    engine: { glue: string; wasm: string };
    canvasKit: {
      full: { glue: string; wasm: string };
    };
    fonts: { defaultSans: string };
    workers: { productFrame: string };
  };
}

export async function emitRuntimeManifests(
  dist: string,
  valleBuildVersion: string,
  runtimeProtocolVersion: number,
  dependencyVersions: { canvasKit: string; mp4box: string },
): Promise<RootRuntimeManifest> {
  if (!valleBuildVersion) throw new Error("Valle runtime build version is required");
  if (!Number.isSafeInteger(runtimeProtocolVersion) || runtimeProtocolVersion <= 0) {
    throw new Error("Valle runtime protocol version must be a positive safe integer");
  }
  const manifestsDir = path.join(dist, "runtime", "manifests");
  await mkdir(manifestsDir, { recursive: true });

  const engine = await manifestOf(dist, "engine", [
    spec("engine-glue", "glue", "runtime/engine/valle_engine.js", "Apache-2.0", "engine-core"),
    spec("engine-wasm", "wasm", "runtime/engine/valle_engine_bg.wasm", "Apache-2.0", "engine-core"),
  ]);
  const apps: PackageRuntimeManifest[] = [];
  const claimedAppScripts = new Set<string>();
  for (const app of ["preview", "studio", "console"] as const) {
    const appDir = path.join(dist, "apps", app);
    const files = (await listFiles(appDir))
      .filter((file) => !file.endsWith(".map"))
      .map((file) => path.relative(dist, file).replaceAll(path.sep, "/"));
    for (const html of files.filter((file) => file.endsWith(".html"))) {
      const source = await Bun.file(path.join(dist, html)).text();
      for (const match of source.matchAll(/<script\b[^>]*\bsrc="([^"]+\.js)"/g)) {
        const script = path.posix.normalize(path.posix.join(path.posix.dirname(html), match[1]));
        files.push(script);
        claimedAppScripts.add(script);
      }
    }
    apps.push(await manifestOf(
      dist,
      `app-${app}`,
      files.map((file, index) => spec(`${app}-${index}`, file.endsWith(".html") ? "html" : "app", file, "Apache-2.0")),
    ));
  }

  const sharedChunks = (await listFiles(path.join(dist, "chunks")))
    .filter((file) => file.endsWith(".js"))
    .map((file) => path.relative(dist, file).replaceAll(path.sep, "/"))
    .filter((file) => !claimedAppScripts.has(file));
  const playerCore = await manifestOf(dist, "player-core", [
    spec("product-frame-worker", "worker", "runtime/workers/product-frame.js", "Apache-2.0"),
    spec("canvaskit-full-glue", "glue", "runtime/canvaskit/canvaskit.js", "BSD-3-Clause", "canvaskit-full", `npm:canvaskit-wasm@${dependencyVersions.canvasKit}`),
    spec("canvaskit-full-wasm", "wasm", "runtime/canvaskit/canvaskit.wasm", "BSD-3-Clause", "canvaskit-full", `npm:canvaskit-wasm@${dependencyVersions.canvasKit}`),
    spec("default-sans", "font", "runtime/fonts/NotoSans-Regular.ttf", "OFL-1.1", undefined, "https://github.com/notofonts/latin-greek-cyrillic"),
    spec("canvaskit-license", "license", "runtime/licenses/canvaskit.txt", "BSD-3-Clause", undefined, `npm:canvaskit-wasm@${dependencyVersions.canvasKit}`),
    spec("mp4box-license", "license", "runtime/licenses/mp4box.txt", "BSD-3-Clause", undefined, `npm:mp4box@${dependencyVersions.mp4box}`),
    spec("valle-third-party-notices", "license", "runtime/licenses/valle-and-third-party.txt", "Apache-2.0 AND MIT AND ISC AND OFL-1.1 AND CC-BY-4.0"),
    ...sharedChunks.map((file, index) => spec(`shared-app-${index}`, "shared", file, "Apache-2.0")),
  ]);

  const packages = [engine, playerCore, ...apps];
  const packageRefs: RootRuntimeManifest["packages"] = [];
  for (const manifest of packages) {
    const manifestPath = `runtime/manifests/${manifest.package}.json`;
    await writeJson(path.join(dist, manifestPath), manifest);
    packageRefs.push({ id: manifest.package, manifest: manifestPath });
  }
  const assets = packages.flatMap((manifest) => manifest.assets).sort((a, b) => a.path.localeCompare(b.path));
  const routes = releaseRoutes(apps);
  const root: RootRuntimeManifest = {
    schemaVersion: RUNTIME_MANIFEST_SCHEMA_VERSION,
    runtimeVersion: valleBuildVersion,
    protocolVersion: runtimeProtocolVersion,
    packages: packageRefs,
    assets,
    assetGroups: [
      { id: "engine-core", glue: "runtime/engine/valle_engine.js", wasm: ["runtime/engine/valle_engine_bg.wasm"] },
      { id: "canvaskit-full", glue: "runtime/canvaskit/canvaskit.js", wasm: ["runtime/canvaskit/canvaskit.wasm"] },
    ],
    workers: [{ id: "product-frame", path: "runtime/workers/product-frame.js" }],
    apps: [
      { id: "preview", html: "apps/preview/index.html" },
      { id: "studio", html: "apps/studio/index.html" },
      { id: "console", html: "apps/console/index.html" },
    ],
    routes,
    runtimeAssets: {
      engine: { glue: "runtime/engine/valle_engine.js", wasm: "runtime/engine/valle_engine_bg.wasm" },
      canvasKit: {
        full: { glue: "runtime/canvaskit/canvaskit.js", wasm: "runtime/canvaskit/canvaskit.wasm" },
      },
      fonts: { defaultSans: "runtime/fonts/NotoSans-Regular.ttf" },
      workers: { productFrame: "runtime/workers/product-frame.js" },
    },
  };
  await writeJson(path.join(dist, "runtime", "manifest.json"), root);
  await assertRuntimeManifest(dist, root);
  return root;
}

export async function assertRuntimeManifest(dist: string, manifest: RootRuntimeManifest): Promise<void> {
  if (manifest.schemaVersion !== RUNTIME_MANIFEST_SCHEMA_VERSION) throw new Error("runtime manifest schema drift");
  if (!Number.isSafeInteger(manifest.protocolVersion) || manifest.protocolVersion <= 0) {
    throw new Error("runtime manifest protocolVersion is invalid");
  }
  const ids = new Set<string>();
  const paths = new Map<string, RuntimeAsset>();
  for (const asset of manifest.assets) {
    if (ids.has(asset.id) || paths.has(asset.path)) throw new Error(`duplicate runtime asset '${asset.id}' / '${asset.path}'`);
    ids.add(asset.id);
    paths.set(asset.path, asset);
    if (!asset.license) throw new Error(`${asset.path}: license is required`);
    assertSafePath(asset.path);
    const bytes = await Bun.file(path.join(dist, asset.path)).bytes();
    if (bytes.byteLength !== asset.bytes || sha256(bytes) !== asset.sha256) {
      throw new Error(`${asset.path}: runtime hash/size drift`);
    }
  }
  for (const group of manifest.assetGroups) {
    const members = [group.glue, ...group.wasm];
    if (group.wasm.length === 0) throw new Error(`${group.id}: wasm group is empty`);
    for (const member of members) {
      if (paths.get(member)?.group !== group.id) throw new Error(`${group.id}: '${member}' is outside its glue+wasm group`);
    }
  }
  for (const worker of manifest.workers) {
    if (paths.get(worker.path)?.role !== "worker") throw new Error(`${worker.id}: worker output is outside the runtime closure`);
  }
  for (const app of manifest.apps) {
    if (paths.get(app.html)?.role !== "html") throw new Error(`${app.id}: app HTML is outside the runtime closure`);
  }
  for (const [route, target] of Object.entries(manifest.routes)) {
    assertSafePath(route);
    if (!paths.has(target)) throw new Error(`${route}: route target '${target}' is outside the runtime closure`);
  }
  for (const runtimePath of flattenRuntimeAssets(manifest.runtimeAssets)) {
    if (!paths.has(runtimePath)) throw new Error(`${runtimePath}: injected runtime URL is outside the closure`);
  }
  const packaged = new Set<string>();
  for (const reference of manifest.packages) {
    const parsed = JSON.parse(await Bun.file(path.join(dist, reference.manifest)).text()) as PackageRuntimeManifest;
    if (parsed.schemaVersion !== manifest.schemaVersion || parsed.package !== reference.id) {
      throw new Error(`${reference.manifest}: package manifest identity drift`);
    }
    for (const asset of parsed.assets) packaged.add(asset.id);
  }
  if (packaged.size !== ids.size || [...ids].some((id) => !packaged.has(id))) {
    throw new Error("root runtime assets do not equal the per-package manifest union");
  }
}

interface AssetSpec {
  id: string;
  role: string;
  path: string;
  license: string;
  group?: string;
  source?: string;
}

function spec(id: string, role: string, assetPath: string, license: string, group?: string, source?: string): AssetSpec {
  return { id, role, path: assetPath, license, group, source };
}

async function manifestOf(dist: string, owner: string, specs: AssetSpec[]): Promise<PackageRuntimeManifest> {
  const assets: RuntimeAsset[] = [];
  for (const value of specs) {
    const bytes = await Bun.file(path.join(dist, value.path)).bytes();
    assets.push({ ...value, owner, bytes: bytes.byteLength, sha256: sha256(bytes) });
  }
  assets.sort((a, b) => a.path.localeCompare(b.path));
  return { schemaVersion: RUNTIME_MANIFEST_SCHEMA_VERSION, package: owner, assets };
}

function releaseRoutes(apps: PackageRuntimeManifest[]): Record<string, string> {
  const routes: Record<string, string> = {};
  for (const manifest of apps) {
    const app = manifest.package.slice("app-".length);
    for (const asset of manifest.assets) {
      const route = asset.role === "html" ? `${app}.html` : path.posix.basename(asset.path);
      if (routes[route]) throw new Error(`release route '${route}' collides`);
      routes[route] = asset.path;
    }
  }
  routes["index.html"] = routes["preview.html"];
  return Object.fromEntries(Object.entries(routes).sort(([a], [b]) => a.localeCompare(b)));
}

function flattenRuntimeAssets(value: RootRuntimeManifest["runtimeAssets"]): string[] {
  return [
    value.engine.glue,
    value.engine.wasm,
    value.canvasKit.full.glue,
    value.canvasKit.full.wasm,
    value.fonts.defaultSans,
    value.workers.productFrame,
  ];
}

function assertSafePath(value: string): void {
  if (!value || value.startsWith("/") || value.split("/").includes("..")) throw new Error(`unsafe runtime path '${value}'`);
}

async function listFiles(dir: string): Promise<string[]> {
  const files: string[] = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const absolute = path.join(dir, entry.name);
    if (entry.isDirectory()) files.push(...await listFiles(absolute));
    else files.push(absolute);
  }
  return files;
}

async function writeJson(file: string, value: unknown): Promise<void> {
  await mkdir(path.dirname(file), { recursive: true });
  await Bun.write(file, `${JSON.stringify(value, null, 2)}\n`);
}

function sha256(bytes: Uint8Array): string {
  return new Bun.CryptoHasher("sha256").update(bytes).digest("hex");
}
