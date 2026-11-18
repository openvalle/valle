// The engine's worker: the wasm engine drawing into the page's canvas, the files it reads fetched
// as it asks for them. One request at a time, answered in order.

import init, { builtInFonts, Engine, type Sound } from "./wasm/valle_web.js";
import type { Code, Opened, Reply, Request, Source } from "./messages.ts";
import { encode } from "./encode.ts";
import type { Reader } from "./video.ts";
import { Media, PCM_PAGE } from "./media.ts";

type Font = { name: string; file: string; url: string; sha256: string; size: number };
/** What the page's `fetch` answered: a status, with the bytes when it is found, or an error. */
type Fetched = { status: number; statusText: string; bytes?: ArrayBuffer } | { error: string };
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
/** Whether files are fetched through the page's `fetch`, and the fetches it is answering, by id. */
let proxied = false;
type MediaResponse =
  | {
      status: number;
      statusText: string;
      headers: [string, string][];
      body?: ReadableStream<Uint8Array>;
    }
  | { error: string };
const mediaFetches = new Map<number, (answer: MediaResponse) => void>();
const fetches = new Map<number, (answer: Fetched) => void>();
let fetchCount = 0;
/** The wasm's memory, whose size is the engine's. */
let memory: WebAssembly.Memory | undefined;
/** The built-in fonts, by their addresses. */
let fonts = new Map<string, Font>();
/** What the page gave in place of fetching a path. */
const given = new Map<string, Source>();
/** Where the engine reads the duration and sample rate of a file's audio track. */
const SOUND = "valle:sound/";
/** The sound of the timeline open: what the audio worklet plays, and what is rendered on its own. */
let playing: Sound | undefined;
let rendering: Sound | undefined;
/** The sample `rendering` gives next, which a render from there need not seek to. */
let next: number | undefined = 0;
/** The audio worklet's port, and the sound it plays: the samples asked for at a time. */
let worklet: MessagePort | undefined;
const CHUNK = 4800;
/** Where the engine asks for a video file's size and length, and its frames for a resource. */
const VIDEO_INFO = "valle:video-info/";
const VIDEO = "valle:video/";
/** The readers of video files: one to probe each file, and one for each resource of a file. */
const readers = new Map<string, { file: string; reader: Promise<Reader | undefined> }>();
const media = new Map<string, Media>();
const pcm = new Map<string, { path: string; data: Float32Array }>();
let pcmBytes = 0;
const PCM_BUDGET = 16 * 1024 * 1024;
const nested = new Map<string, { sound: Sound | undefined; next: number | undefined }>();
/** The composition open. */
let composition: Opened | undefined;
let shownFrame = 0;
let playingGen: number | undefined;
const exportAbort = new AbortController();

async function handle(request: Request, id: number): Promise<unknown> {
  if (request.type === "start") {
    ({ memory } = await init({
      module_or_path: new URL("./valle_web_bg.wasm", import.meta.url),
    }));
    fonts = new Map((JSON.parse(builtInFonts()) as Font[]).map((f) => [f.url, f]));
    base = request.base;
    fontBase = request.fonts;
    proxied = request.proxied;
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
    case "update-timeline": {
      const previous = new Map<string, Source | undefined>();
      const changed = Object.entries(request.files);
      for (const [path, file] of changed) {
        previous.set(path, given.get(path));
        invalidate(path);
        given.set(path, file);
      }
      let candidate: Sound | undefined;
      let installed = false;
      try {
        const info = JSON.parse(
          await fetching(() =>
            e.prepare(request.entry, JSON.stringify(request.inputs), request.timeline),
          ),
        ) as Opened;
        candidate = await openSound(e, request.entry, request.timeline);
        const [n, d = 1] = info.fps.split("/").map(Number);
        const index = Math.max(
          0,
          Math.min(info.frames - 1, Math.floor((request.seconds * n) / d + 1e-9)),
        );
        e.begin_update();
        installed = true;
        await fetching(() => e.frame(index));
        await e.drawn();
        e.commit();
        e.give(request.entry, new TextEncoder().encode(request.timeline));
        given.set(request.entry, new Blob([request.timeline]));
        clearNested();
        useSound(candidate);
        candidate = undefined;
        composition = info;
        shownFrame = index;
        return { info, index, sound: playing ? { length: playing.length() } : null };
      } catch (error) {
        e.discard();
        candidate?.free();
        for (const [path, old] of previous) {
          invalidate(path);
          if (old === undefined) given.delete(path);
          else given.set(path, old);
        }
        if (installed) {
          await fetching(() => e.frame(shownFrame));
          await e.drawn();
        }
        throw error;
      }
    }
    case "export": {
      const sound = request.options.audio ? rendering?.fresh() : undefined;
      try {
        return await encode(
          request.options,
          composition!,
          async (index, planes) => {
            await fetching(() => e.video_frame(index, planes));
          },
          e,
          sound ? (count) => renderSound(sound, count) : undefined,
          request.destination,
          (done) => self.postMessage({ progress: id, done }),
          exportAbort.signal,
        );
      } finally {
        sound?.free();
      }
    }
    case "frame":
      await fetching(() => e.frame(request.index));
      await e.drawn();
      shownFrame = request.index;
      return { memory: memory?.buffer.byteLength ?? 0 };
    case "pixels":
      return await e.pixels();
    case "still":
      await fetching(() => e.render(request.index));
      return await e.pixels();
    case "sound":
      useSound(await openSound(e, request.entry));
      return playing ? { length: playing.length() } : null;
    case "render-sound": {
      if (!rendering) {
        throw new Failure("scene", "what is open has no sound");
      }
      const at = request.sample === next ? undefined : request.sample;
      next = undefined;
      const data = await renderSound(rendering, request.samples, at);
      next = request.sample + request.samples;
      return data;
    }
    case "worklet":
      worklet = request.port;
      worklet.onmessage = (event: MessageEvent<{ gen: number; sample?: number }>) => {
        const { gen, sample } = event.data;
        const sound = playing;
        queue = queue.then(async () => {
          if (!sound || sound !== playing || (sample === undefined && playingGen !== gen)) return;
          try {
            const data = await renderSound(sound, CHUNK, sample);
            playingGen = gen;
            worklet!.postMessage({ gen, data }, [data.buffer]);
          } catch (error) {
            playingGen = undefined;
            self.postMessage({ soundError: String(error) });
          }
        });
      };
      return undefined;
    case "close":
      useSound(undefined);
      clearNested();
      e.close();
      return undefined;
    case "destroy":
      useSound(undefined);
      clearNested();
      await Promise.all([...media.keys()].map(dropMedia));
      e.destroy();
      engine = undefined;
      return undefined;
  }
}

function useSound(sound: Sound | undefined) {
  playing?.free();
  rendering?.free();
  playing = sound;
  rendering = sound?.fresh();
  playingGen = undefined;
  next = 0;
}

async function openSound(e: Engine, entry: string, timeline?: string): Promise<Sound | undefined> {
  const status = JSON.parse(await fetching(() => e.sound(entry, timeline))) as { status: string };
  return status.status === "ok" ? e.take_sound() : undefined;
}

function clearNested() {
  for (const child of nested.values()) child.sound?.free();
  nested.clear();
  for (const [key, page] of pcm) {
    if (key.startsWith("timeline:")) {
      pcm.delete(key);
      pcmBytes -= page.data.byteLength;
    }
  }
}

/** One request pins only the pages touched by a short output window. */
async function renderSound(sound: Sound, count: number, seek?: number): Promise<Float32Array> {
  const out = new Float32Array(count * 2);
  try {
    for (let done = 0; done < count; done += CHUNK) {
      const samples = Math.min(CHUNK, count - done);
      const at = done === 0 ? seek : undefined;
      const pages = JSON.parse(sound.pages(at, samples)) as [number, number, string, boolean][];
      for (const [source, index, path, timeline] of pages) {
        sound.give_page(source, index, await audioPage(path, index, timeline));
      }
      if (at !== undefined) sound.seek(at);
      out.set(sound.render(samples), done * 2);
    }
  } finally {
    sound.clear_pages();
  }
  return out;
}

async function audioPage(path: string, index: number, timeline: boolean): Promise<Float32Array> {
  const key = (timeline ? "timeline:" : "file:") + JSON.stringify([path, index]);
  const hit = pcm.get(key);
  if (hit) {
    pcm.delete(key);
    pcm.set(key, hit);
    return hit.data;
  }
  let data: Float32Array;
  if (timeline) {
    let child = nested.get(path);
    if (!child) {
      child = { sound: await openSound(engine!, path), next: 0 };
      nested.set(path, child);
    }
    const at = index * PCM_PAGE;
    const seek = child.next === at ? undefined : at;
    child.next = undefined;
    data = child.sound
      ? await renderSound(child.sound, PCM_PAGE, seek)
      : new Float32Array(2 * PCM_PAGE);
    child.next = at + PCM_PAGE;
  } else {
    try {
      const track = await mediaFor(path).audio();
      if (!track) throw new Error("no audio track");
      data = await track.page(index);
    } catch (error) {
      dropMedia(path);
      throw new Failure("file", `${path}: ${String(error)}`);
    }
  }
  while (pcm.size > 0 && pcmBytes + data.byteLength > PCM_BUDGET) {
    const oldest = pcm.keys().next().value!;
    pcmBytes -= pcm.get(oldest)!.data.byteLength;
    pcm.delete(oldest);
  }
  pcm.set(key, { path, data });
  pcmBytes += data.byteLength;
  return data;
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
    for (let i = 0; i < status.files.length; i += 4) {
      // Finish the whole batch before allowing an update to roll back its source map.
      const batch = await Promise.allSettled(status.files.slice(i, i + 4).map(load));
      const failed = batch.find((r) => r.status === "rejected");
      if (failed?.status === "rejected") throw failed.reason;
      prune();
    }
  }
}

/** Gives the engine the file at `path`, or says there is none. */
async function load(path: string): Promise<void> {
  const e = engine!;
  if (path.startsWith(SOUND)) {
    const file = path.slice(SOUND.length);
    try {
      const info = await mediaFor(file).audio();
      if (info?.frames) {
        e.give(path, new TextEncoder().encode(JSON.stringify([info.frames, info.rate])));
      } else {
        e.absent(path);
      }
    } catch (error) {
      dropMedia(file);
      throw new Failure("file", `${file}: ${String(error)}`);
    }
    return;
  }
  if (path.startsWith(VIDEO_INFO)) {
    const file = path.slice(VIDEO_INFO.length);
    try {
      const probed = await mediaFor(file).probe();
      if (probed) e.give(path, new TextEncoder().encode(JSON.stringify(probed)));
      else e.absent(path);
    } catch (error) {
      dropMedia(file);
      throw new Failure("file", `${file}: ${String(error)}`);
    }
    return;
  }
  if (path.startsWith(VIDEO)) {
    // `<resource>@<seconds>/<file>`
    const rest = path.slice(VIDEO.length);
    const [at, slash] = [rest.indexOf("@"), rest.indexOf("/")];
    const file = rest.slice(slash + 1);
    const key = `${rest.slice(0, at)}/${file}`;
    const reader = await opened(key, file);
    const seconds = Number(rest.slice(at + 1, slash));
    const pixels = await reader!.frame(seconds).catch((error: unknown) => {
      dropReader(key);
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

function mediaFor(path: string): Media {
  let input = media.get(path);
  if (input) {
    media.delete(path);
  } else {
    const source = given.get(path);
    const file =
      source instanceof Blob
        ? source
        : typeof source === "string" || source === undefined
          ? new URL(source ?? path, base).href
          : new Blob([source as BlobPart]);
    input = new Media(file, proxied ? streamMedia : undefined);
  }
  media.set(path, input);
  return input;
}

/** Each clip cursor is separate, but the input and compressed-byte cache are shared. */
function opened(key: string, path: string): Promise<Reader | undefined> {
  const input = mediaFor(path);
  let entry = readers.get(key);
  if (entry) {
    readers.delete(key);
  } else {
    const reader = input.reader().catch((error: unknown) => {
      readers.delete(key);
      throw new Failure("file", `${path}: ${String(error)}`);
    });
    entry = { file: path, reader };
  }
  readers.set(key, entry);
  return entry.reader;
}

function dropReader(key: string) {
  void readers.get(key)?.reader.then(
    (r) => r?.dispose(),
    () => {},
  );
  readers.delete(key);
}

function dropMedia(path: string) {
  for (const [key, reader] of readers) {
    if (reader.file === path) dropReader(key);
  }
  const input = media.get(path);
  media.delete(path);
  return input?.dispose().catch((error) => console.warn("removing temporary media", error));
}

function prune() {
  while (readers.size > 16) dropReader(readers.keys().next().value!);
  while (media.size > 24) dropMedia(media.keys().next().value!);
  while (nested.size > 16) {
    const path = nested.keys().next().value!;
    nested.get(path)?.sound?.free();
    nested.delete(path);
  }
}

function invalidate(path: string) {
  engine!.invalidate(path);
  dropMedia(path);
  for (const [key, page] of pcm) {
    if (page.path === path) {
      pcmBytes -= page.data.byteLength;
      pcm.delete(key);
    }
  }
}

const streamMedia: typeof fetch = (input, init) => {
  const id = fetchCount++;
  const { signal, headers, url } = new Request(input, init);
  signal.throwIfAborted();
  return new Promise<Response>((resolve, reject) => {
    const cancel = () => {
      mediaFetches.delete(id);
      self.postMessage({ cancelMedia: id });
      reject(signal.reason);
    };
    signal.addEventListener("abort", cancel, { once: true });
    mediaFetches.set(id, (answer) => {
      const finish = () => signal.removeEventListener("abort", cancel);
      if ("error" in answer) {
        finish();
        reject(new Error(answer.error));
        return;
      }
      if (!answer.body) {
        finish();
        resolve(new Response(null, answer));
        return;
      }
      const reader = answer.body.getReader();
      const stream = new ReadableStream<Uint8Array>(
        {
          async pull(controller) {
            try {
              const chunk = await reader.read();
              signal.throwIfAborted();
              if (chunk.done) {
                finish();
                controller.close();
              } else {
                controller.enqueue(chunk.value);
              }
            } catch (error) {
              finish();
              controller.error(error);
            }
          },
          cancel(reason) {
            finish();
            return reader.cancel(reason);
          },
        },
        { highWaterMark: 0 },
      );
      resolve(new Response(stream, answer));
    });
    self.postMessage({ media: id, url, headers: [...headers] });
  });
};

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
  const response = await get(url);
  if ("error" in response) {
    throw new Failure("file", `${url}: ${response.error}`);
  }
  // An import tries its extensions in turn, so a file that is not there is an answer.
  if (response.status === 404) {
    if (font) {
      throw new Failure("file", `${url}: the built-in font ${font.name} is not there`);
    }
    return undefined;
  }
  if (!response.bytes) {
    throw new Failure("file", `${url}: ${response.status} ${response.statusText}`);
  }
  const bytes = new Uint8Array(response.bytes);
  if (font) {
    const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
    const hex = Array.from(digest, (b) => b.toString(16).padStart(2, "0")).join("");
    if (hex !== font.sha256) {
      throw new Failure("file", `${url}: not the built-in font ${font.name} (SHA-256 ${hex})`);
    }
  }
  return bytes;
}

/** `url` fetched, through the page's `fetch` where it gave one. */
async function get(url: string): Promise<Fetched> {
  if (proxied) {
    const id = fetchCount++;
    return new Promise((resolve) => {
      fetches.set(id, resolve);
      self.postMessage({ fetch: id, url });
    });
  }
  try {
    const response = await fetch(url);
    const { status, statusText } = response;
    return { status, statusText, bytes: response.ok ? await response.arrayBuffer() : undefined };
  } catch (error) {
    return { error: String(error) };
  }
}

// What fails outside a request stops the worker, which the player reports.
self.onunhandledrejection = (event: PromiseRejectionEvent) => {
  throw event.reason instanceof Error ? event.reason : new Error(String(event.reason));
};

let queue = Promise.resolve();
self.onmessage = (
  event: MessageEvent<
    | ({ id: number } & Request)
    | ({ media: number } & MediaResponse)
    | { cancelExport: true }
    | ({ fetched: number } & Fetched)
  >,
) => {
  if ("cancelExport" in event.data || ("type" in event.data && event.data.type === "destroy")) {
    for (const input of media.values()) input.cancelDownload();
  }
  if ("cancelExport" in event.data) {
    exportAbort.abort();
    return;
  }
  if ("fetched" in event.data) {
    const { fetched, ...answer } = event.data;
    fetches.get(fetched)?.(answer);
    fetches.delete(fetched);
    return;
  }
  if ("media" in event.data) {
    const { media: id, ...answer } = event.data;
    const receive = mediaFetches.get(id);
    if (receive) receive(answer);
    else if ("body" in answer) void answer.body?.cancel().catch(() => {});
    mediaFetches.delete(id);
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
    prune();
    const value = reply.ok ? reply.value : undefined;
    const buffer = ArrayBuffer.isView(value) ? value.buffer : value;
    self.postMessage(reply, buffer instanceof ArrayBuffer ? [buffer] : []);
  });
};
