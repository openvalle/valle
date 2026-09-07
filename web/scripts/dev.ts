import path from "node:path";

const root = path.resolve(import.meta.dir, "..");
const build = Bun.spawn(["bun", "run", "build:runtime"], { cwd: root, stdout: "inherit", stderr: "inherit" });
if (await build.exited !== 0) throw new Error("initial web build failed");

const appRoot = path.join(root, "dist", "apps", "preview");
const server = Bun.serve({
  port: Number(Bun.env.PORT ?? 9527),
  async fetch(request) {
    const url = new URL(request.url);
    const relative = url.pathname === "/" ? "index.html" : url.pathname.slice(1);
    const file = path.resolve(appRoot, relative);
    if (file !== appRoot && !file.startsWith(`${appRoot}${path.sep}`)) return new Response("Not found", { status: 404 });
    try {
      // Missing paths and directories return false; the outer catch handles other IO errors.
      if (!(await Bun.file(file).exists())) return new Response("Not found", { status: 404 });
      return new Response(Bun.file(file));
    } catch {
      return new Response("Not found", { status: 404 });
    }
  },
});

console.log(`Valle Web preview: ${server.url}`);
