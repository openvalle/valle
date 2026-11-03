// The engine's worker: the wasm engine drawing into the page's canvas, the files it reads fetched
// as it asks for them. One request at a time, answered in order.

import init, { builtInFonts, Engine } from "./wasm/valle_web.js";
import type { Code, Reply, Request, Source } from "./messages.ts";

type Font = { name: string; file: string; url: string; sha256: string; size: number };
type Status = { status: "ok" } | { status: "missing"; files: string[] };

class Failure extends Error {
  constructor(
    readonly code: Code,
    message: string,
  ) {
    super(message);
  }
}

let engine: Engine | undefined;
let base = "";
let fontBase: string | undefined;
/** The built-in fonts, by their addresses. */
let fonts = new Map<string, Font>();
/** What the page gave in place of fetching a path. */
const given = new Map<string, Source>();

async function handle(request: Request): Promise<unknown> {
  if (request.type === "start") {
    await init({ module_or_path: new URL("./valle_web_bg.wasm", import.meta.url) });
    fonts = new Map((JSON.parse(builtInFonts()) as Font[]).map((f) => [f.url, f]));
    base = request.base;
    fontBase = request.fonts;
    if (!("gpu" in navigator)) {
      throw new Failure("no-webgpu", "this browser has no WebGPU");
    }
    engine = await Engine.start(request.canvas).catch((e: unknown) => {
      throw new Failure("no-webgpu", String(e));
    });
    return undefined;
  }
  if (request.type === "give") {
    for (const [path, source] of Object.entries(request.files)) {
      given.set(path, source);
    }
    return undefined;
  }
  const e = engine;
  if (!e) {
    throw new Failure("scene", "the engine has not started");
  }
  switch (request.type) {
    case "open":
      return JSON.parse(
        await fetching(() => e.open(request.entry, JSON.stringify(request.inputs))),
      );
    case "frame":
      await fetching(() => e.frame(request.index));
      await e.drawn();
      return undefined;
    case "pixels":
      return await e.pixels();
    case "close":
      e.close();
      return undefined;
    case "destroy":
      e.destroy();
      engine = undefined;
      return undefined;
  }
}

/** Runs `step` until it has every file it reads, fetching those it asks for in between. */
async function fetching(step: () => string): Promise<string> {
  for (;;) {
    let text: string;
    try {
      text = step();
    } catch (error) {
      throw new Failure("scene", String(error));
    }
    const status = JSON.parse(text) as Status;
    if (status.status !== "missing") {
      return text;
    }
    await Promise.all(status.files.map(load));
  }
}

/** Gives the engine the file at `path`, or says there is none. */
async function load(path: string): Promise<void> {
  const e = engine!;
  const bytes = await read(path);
  if (bytes) {
    e.give(path, bytes);
  } else {
    e.absent(path);
  }
}

/** The bytes of the file at `path`, or nothing where there is none. */
async function read(path: string): Promise<Uint8Array | undefined> {
  const source = given.get(path);
  if (source instanceof Uint8Array) {
    return source;
  }
  if (source instanceof ArrayBuffer) {
    return new Uint8Array(source);
  }
  if (source instanceof Blob) {
    return new Uint8Array(await source.arrayBuffer());
  }
  const font = fonts.get(path);
  const url =
    source ?? (font && fontBase ? new URL(font.file, fontBase).href : new URL(path, base).href);
  let response: Response;
  try {
    response = await fetch(url);
  } catch (error) {
    throw new Failure("file", `${url}: ${String(error)}`);
  }
  // An import tries its extensions in turn, so a file that is not there is an answer.
  if (response.status === 404) {
    if (font) {
      throw new Failure("file", `${url}: the built-in font ${font.name} is not there`);
    }
    return undefined;
  }
  if (!response.ok) {
    throw new Failure("file", `${url}: ${response.status} ${response.statusText}`);
  }
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (font) {
    const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
    const hex = Array.from(digest, (b) => b.toString(16).padStart(2, "0")).join("");
    if (hex !== font.sha256) {
      throw new Failure("file", `${url}: not the built-in font ${font.name} (SHA-256 ${hex})`);
    }
  }
  return bytes;
}

let queue = Promise.resolve();
self.onmessage = (event: MessageEvent<{ id: number } & Request>) => {
  const { id, ...request } = event.data;
  queue = queue.then(async () => {
    let reply: Reply;
    try {
      reply = { id, ok: true, value: await handle(request as Request) };
    } catch (error) {
      const code = error instanceof Failure ? error.code : "scene";
      const message = error instanceof Error ? error.message : String(error);
      reply = { id, ok: false, code, message };
    }
    const value = reply.ok ? reply.value : undefined;
    self.postMessage(reply, value instanceof Uint8Array ? [value.buffer] : []);
  });
};
