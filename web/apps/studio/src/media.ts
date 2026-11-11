// Pictures of the folder's media, for the timeline's clips and the media browser: a video's
// frames, read by a <video> kept off the page, and a sound's peaks. Each is made once and
// kept; a file the browser cannot read simply has none.

/** Pixels a frame is kept at, high. */
const HEIGHT = 54;
/** Peaks kept for a second of sound. */
export const PEAKS = 50;
/** The largest sound decoded for its peaks, in bytes. */
const LARGEST = 96 << 20;

type Reel = { video: HTMLVideoElement; queue: Promise<void>; broken: boolean };
type Pending<T> = { value?: T; listeners?: Set<() => void> };

const reels = new Map<string, Reel>();
const frames = new Map<string, Pending<string>>();
let revision = 0;

// Browsers can share a decoded media resource between video elements, even after the old
// element is released. A changed file must get a new media URL as well as a new cache entry.
function source(url: string): string {
  return `${url}${url.includes("?") ? "&" : "?"}revision=${revision}`;
}

/** Forget a changed file, or all files when leaving a project. In-flight reads cannot put
 * old results back into the cache. */
export function invalidate(url?: string): void {
  revision++;
  for (const [key, reel] of reels) {
    if (url !== undefined && key !== url) continue;
    reels.delete(key);
    reel.broken = true;
    reel.video.pause();
    reel.video.removeAttribute("src");
    reel.video.load();
  }
  for (const key of frames.keys()) {
    if (url === undefined || key.startsWith(`${url}#`)) frames.delete(key);
  }
  for (const [key, sound] of sounds) {
    if (url !== undefined && key !== url) continue;
    sounds.delete(key);
    sound.abort.abort();
  }
}

/** The frame of the video at `url` nearest `time`, once made; `done` says when it is. */
export function frame(url: string, time: number, done: () => void): string | undefined {
  const at = Math.max(0, Math.round(time * 2) / 2);
  const key = `${url}#${at}`;
  const known = frames.get(key);
  if (known) {
    known.listeners?.add(done);
    return known.value;
  }
  const pending: Pending<string> = { listeners: new Set([done]) };
  frames.set(key, pending);
  let reel = reels.get(url);
  if (!reel) {
    const video = document.createElement("video");
    video.muted = true;
    video.preload = "auto";
    video.src = source(url);
    reel = { video, queue: Promise.resolve(), broken: false };
    reels.set(url, reel);
  }
  const { video } = reel;
  const own = reel;
  reel.queue = reel.queue.then(async () => {
    if (own.broken) {
      pending.listeners = undefined;
      return;
    }
    try {
      const picture = await shoot(video, at);
      if (picture && frames.get(key) === pending) {
        pending.value = picture;
        for (const listener of pending.listeners ?? []) listener();
      }
    } catch {
      own.broken = true;
    } finally {
      pending.listeners = undefined;
    }
  });
  return undefined;
}

async function shoot(video: HTMLVideoElement, at: number): Promise<string | undefined> {
  if (video.readyState < 1) await heard(video, "loadedmetadata");
  const time = Math.min(at, Math.max(0, video.duration - 0.05));
  if (Math.abs(video.currentTime - time) > 1e-3) {
    video.currentTime = time;
    await heard(video, "seeked");
  } else if (video.readyState < 2) {
    await heard(video, "loadeddata");
  }
  if (!video.videoWidth) return undefined;
  const canvas = document.createElement("canvas");
  canvas.height = HEIGHT;
  canvas.width = Math.max(1, Math.round((HEIGHT * video.videoWidth) / video.videoHeight));
  canvas.getContext("2d")!.drawImage(video, 0, 0, canvas.width, canvas.height);
  return canvas.toDataURL("image/jpeg", 0.7);
}

/** Once `media` says `name`; an error or 10 s of silence fails. */
function heard(media: HTMLMediaElement, name: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const finish = (error?: Error) => {
      clearTimeout(timer);
      media.removeEventListener(name, said);
      media.removeEventListener("error", failed);
      media.removeEventListener("emptied", failed);
      if (error) reject(error);
      else resolve();
    };
    const said = () => finish();
    const failed = () => finish(new Error("unreadable"));
    const timer = setTimeout(failed, 10000);
    media.addEventListener(name, said);
    media.addEventListener("error", failed);
    media.addEventListener("emptied", failed);
  });
}

/** A sound's loudest sample in each 1/50 s, from 0 to 1. */
export type Peaks = Float32Array;

const sounds = new Map<string, Pending<Peaks> & { abort: AbortController }>();

/** The peaks of the sound at `url`, once made; `done` says when they are. */
export function peaks(url: string, done: () => void): Peaks | undefined {
  const known = sounds.get(url);
  if (known) {
    known.listeners?.add(done);
    return known.value;
  }
  const pending: Pending<Peaks> & { abort: AbortController } = {
    listeners: new Set([done]),
    abort: new AbortController(),
  };
  sounds.set(url, pending);
  listen(url, pending.abort.signal)
    .then(
      (made) => {
        if (made && sounds.get(url) === pending) {
          pending.value = made;
          for (const listener of pending.listeners ?? []) listener();
        }
      },
      () => {},
    )
    .finally(() => {
      pending.listeners = undefined;
    });
  return undefined;
}

async function listen(url: string, signal: AbortSignal): Promise<Peaks | undefined> {
  const response = await fetch(source(url), { signal });
  if (!response.ok || Number(response.headers.get("content-length")) > LARGEST) {
    await response.body?.cancel();
    return undefined;
  }
  // A low rate keeps the decoded sound small; peaks need no more.
  const context = new OfflineAudioContext(1, 1, 8000);
  const sound = await context.decodeAudioData(await response.arrayBuffer());
  const block = sound.sampleRate / PEAKS;
  const made = new Float32Array(Math.ceil(sound.length / block));
  for (let channel = 0; channel < sound.numberOfChannels; channel++) {
    const samples = sound.getChannelData(channel);
    for (let i = 0; i < samples.length; i++) {
      const n = Math.floor(i / block);
      const level = Math.abs(samples[i]);
      if (level > made[n]) made[n] = level;
    }
  }
  return made;
}

/** The shape of `sound` from `start` to `end` seconds, as the path of an SVG `columns` wide and
 * 2 high, mirrored about its middle. */
export function wave(sound: Peaks, start: number, end: number): { d: string; columns: number } {
  const columns = Math.max(2, Math.min(400, Math.ceil((end - start) * PEAKS)));
  const levels: number[] = [];
  for (let i = 0; i < columns; i++) {
    const from = Math.floor((start + ((end - start) * i) / columns) * PEAKS);
    const to = Math.max(
      from + 1,
      Math.floor((start + ((end - start) * (i + 1)) / columns) * PEAKS),
    );
    let level = 0;
    for (let n = Math.max(0, from); n < to && n < sound.length; n++) {
      level = Math.max(level, sound[n]);
    }
    levels.push(Math.min(1, level));
  }
  const upper = levels.map((level, i) => `${i} ${(1 - level).toFixed(3)}`);
  const lower = levels.map((level, i) => `${i} ${(1 + level).toFixed(3)}`).reverse();
  return { d: `M${[...upper, `${columns} 1`, ...lower].join("L")}Z`, columns };
}
