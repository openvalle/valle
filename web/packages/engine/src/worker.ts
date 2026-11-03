// The engine's worker: the wasm engine drawing into the page's canvas, the files it reads fetched
// as it asks for them. One request at a time, answered in order.

import init, { builtInFonts, Engine, type Sound } from "./wasm/valle_web.js";
import type { Code, Opened, Reply, Request, Source } from "./messages.ts";
import { encode } from "./export.ts";
import { Reader } from "./video.ts";

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
/** Where the engine reads a file's sound, which the page decodes. */
const SOUND = "valle:sound/";
/** The sound of the timeline open: what the audio worklet plays, and what is rendered on its own. */
let playing: Sound | undefined;
let rendering: Sound | undefined;
/** The sample `rendering` gives next, which a render from there need not seek to. */
let next = 0;
/** The audio worklet's port, and the sound it plays: the samples asked for at a time. */
let worklet: MessagePort | undefined;
const CHUNK = 4800;
/** Where the engine asks for a video file's size and length, and its frames for a resource. */
const VIDEO_INFO = "valle:video-info/";
const VIDEO = "valle:video/";
/** The readers of video files: one to probe each file, and one for each resource of a file. */
const readers = new Map<string, Promise<Reader | undefined>>();
/** The composition open, and whether the export running was cancelled. */
let composition: Opened | undefined;
let cancelled = false;
/** Sounds the page is decoding, by id. */
const decoding = new Map<number, (pcm: Float32Array | null) => void>();
let decodes = 0;

async function handle(request: Request, id: number): Promise<unknown> {
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
      composition = JSON.parse(
        await fetching(() => e.open(request.entry, JSON.stringify(request.inputs))),
      ) as Opened;
      return composition;
    case "export": {
      const { width, height, fps, frames } = composition!;
      const [n, d = "1"] = fps.split("/");
      cancelled = false;
      const sound = rendering?.fresh();
      try {
        return await encode(
          { width, height, rate: [Number(n), Number(d)], frames, ...request.options },
          e,
          async (index) => {
            await fetching(() => e.render(index));
          },
          sound,
          (done) => self.postMessage({ progress: id, done }),
          () => cancelled,
        );
      } catch (error) {
        throw cancelled ? new Failure("cancelled", "the export was cancelled") : error;
      } finally {
        sound?.free();
      }
    }
    case "frame":
      await fetching(() => e.frame(request.index));
      await e.drawn();
      return undefined;
    case "pixels":
      return await e.pixels();
    case "sound": {
      const status = JSON.parse(await fetching(() => e.sound(request.entry))) as { status: string };
      forget();
      playing = status.status === "ok" ? e.take_sound() : undefined;
      rendering = playing?.fresh();
      next = 0;
      return playing ? { length: playing.length() } : null;
    }
    case "render-sound": {
      if (!rendering) {
        throw new Failure("scene", "what is open has no sound");
      }
      if (request.sample !== next) {
        rendering.seek(request.sample);
      }
      next = request.sample + request.samples;
      return rendering.render(request.samples);
    }
    case "worklet":
      worklet = request.port;
      worklet.onmessage = (event: MessageEvent<{ gen: number; sample?: number }>) => {
        const { gen, sample } = event.data;
        if (!playing) {
          return;
        }
        if (sample !== undefined) {
          playing.seek(sample);
        }
        const data = playing.render(CHUNK);
        worklet!.postMessage({ gen, data }, [data.buffer]);
      };
      return undefined;
    case "close":
      forget();
      e.close();
      return undefined;
    case "destroy":
      for (const reader of readers.values()) {
        void reader.then((r) => r?.dispose());
      }
      readers.clear();
      e.destroy();
      engine = undefined;
      return undefined;
  }
}

/** Lets the sound of what was open go. */
function forget() {
  playing?.free();
  rendering?.free();
  playing = rendering = undefined;
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
  if (path.startsWith(SOUND)) {
    const file = await read(path.slice(SOUND.length));
    const pcm = file && (await decode(file.slice()));
    if (pcm) {
      e.give(path, new Uint8Array(pcm.buffer));
    } else {
      e.absent(path);
    }
    return;
  }
  if (path.startsWith(VIDEO_INFO)) {
    const file = path.slice(VIDEO_INFO.length);
    const reader = await opened(file, file);
    if (reader) {
      const probed = await reader.probe().catch((error: unknown) => {
        throw new Failure("file", `${file}: ${String(error)}`);
      });
      e.give(path, new TextEncoder().encode(JSON.stringify(probed)));
    } else {
      e.absent(path);
    }
    return;
  }
  if (path.startsWith(VIDEO)) {
    // `<resource>@<seconds>/<file>`
    const rest = path.slice(VIDEO.length);
    const [at, slash] = [rest.indexOf("@"), rest.indexOf("/")];
    const file = rest.slice(slash + 1);
    const reader = await opened(`${rest.slice(0, at)}/${file}`, file);
    const seconds = Number(rest.slice(at + 1, slash));
    const pixels = await reader!.frame(seconds).catch((error: unknown) => {
      throw new Failure("file", `${file} at ${seconds} s: ${String(error)}`);
    });
    e.give(path, pixels);
    return;
  }
  const bytes = await read(path);
  if (bytes) {
    e.give(path, bytes);
  } else {
    e.absent(path);
  }
}

/** The reader `key` names of the video file at `path`, opened the first time. */
function opened(key: string, path: string): Promise<Reader | undefined> {
  let reader = readers.get(key);
  if (!reader) {
    const source = given.get(path);
    const file =
      source === undefined || typeof source === "string"
        ? (source ?? new URL(path, base).href)
        : new Blob([source]);
    reader = Reader.open(file).catch((error: unknown) => {
      readers.delete(key);
      throw new Failure("file", `${path}: ${String(error)}`);
    });
    readers.set(key, reader);
  }
  return reader;
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

/** A file's sound as the page decodes it, a worker cannot: interleaved stereo at 48 000 a
 * second, or none where it has no sound. */
function decode(file: Uint8Array): Promise<Float32Array | null> {
  const id = decodes++;
  return new Promise((resolve) => {
    decoding.set(id, resolve);
    self.postMessage({ decode: id, file }, [file.buffer]);
  });
}

// What fails outside a request stops the worker, which the player reports.
self.onunhandledrejection = (event: PromiseRejectionEvent) => {
  throw event.reason instanceof Error ? event.reason : new Error(String(event.reason));
};

let queue = Promise.resolve();
self.onmessage = (
  event: MessageEvent<
    ({ id: number } & Request) | { decoded: number; pcm: Float32Array | null } | { cancel: true }
  >,
) => {
  if ("cancel" in event.data) {
    cancelled = true;
    return;
  }
  if ("decoded" in event.data) {
    const { decoded, pcm } = event.data;
    decoding.get(decoded)?.(pcm);
    decoding.delete(decoded);
    return;
  }
  const { id, ...request } = event.data;
  queue = queue.then(async () => {
    let reply: Reply;
    try {
      reply = { id, ok: true, value: await handle(request as Request, id) };
    } catch (error) {
      const code = error instanceof Failure ? error.code : "scene";
      const message = error instanceof Error ? error.message : String(error);
      reply = { id, ok: false, code, message };
    }
    const value = reply.ok ? reply.value : undefined;
    const buffer = ArrayBuffer.isView(value) ? value.buffer : value;
    self.postMessage(reply, buffer instanceof ArrayBuffer ? [buffer] : []);
  });
};
