// Exporting in the browser: what a player has open, as it is bound now, rendered again by a
// worker of its own into an MP4, so playing, seeking and the volume do not touch it.
//
//   import { exportBrowserVideo } from "@openvalle/engine/export";
//   const { blob } = await exportBrowserVideo(player, { width: 1280, height: 720 });

import { Connection, PlayerError, spawns } from "./connection.ts";
import type { Player } from "./index.ts";
import { exportSchedule, type FileDestination, type Opened } from "./messages.ts";

export type ExportVideoOptions = {
  /** Output pixels, rendered directly without relayout, up to UHD 8K. The composition's size
   * by default; fitted whole and centred on black. Given one, the other keeps its shape. */
  width?: number;
  height?: number;
  /** Frames a second; the composition's by default. At another rate, each frame is the
   * composition's frame at its time. */
  frameRate?: number;
  /** Bits a second of the video; by default 0.15 a pixel a frame, as the native engine's. */
  videoBitrate?: number;
  /** Whether the sound is in it, when there is any: by default it is. */
  includeAudio?: boolean;
  /** Cancels the export, which then fails with `cancelled`. */
  signal?: AbortSignal;
  onProgress?: (progress: { framesCompleted: number; frameCount: number }) => void;
};

export type ExportFileTarget = { type: "opfs" } | { type: "file"; handle: FileSystemFileHandle };
export type ExportedFile = {
  fileHandle: FileSystemFileHandle;
  /** Removes a temporary OPFS result when it is no longer needed. */
  cleanup?: () => Promise<void>;
};

/** Exports a snapshot as MP4, independently of playback or subsequent edits. */
export function exportBrowserVideo(
  player: Player,
  options?: ExportVideoOptions & { target?: undefined },
): Promise<{ blob: Blob }>;
export function exportBrowserVideo(
  player: Player,
  options: ExportVideoOptions & { target: ExportFileTarget },
): Promise<ExportedFile>;
export async function exportBrowserVideo(
  player: Player,
  options: ExportVideoOptions & { target?: ExportFileTarget } = {},
): Promise<{ blob: Blob } | ExportedFile> {
  const open = spawns.get(player);
  if (!open) {
    throw new PlayerError("scene", "the player was closed");
  }
  // Capture the entry and the resource bindings before starting a worker or asking for a file.
  const spawn = { ...open, inputs: structuredClone(open.inputs), given: { ...open.given } };
  const { signal, onProgress, includeAudio = true, target } = options;
  signal?.throwIfAborted();

  let destination: FileDestination | undefined;
  let cleanup: (() => Promise<void>) | undefined;
  if (target?.type === "opfs") {
    if (typeof navigator.storage?.getDirectory !== "function") {
      throw new PlayerError("file", "this browser has no origin-private file storage");
    }
    const root = await navigator.storage.getDirectory();
    const directory = await root.getDirectoryHandle("valle-exports", { create: true });
    const name = `${crypto.randomUUID()}.mp4`;
    const handle = await directory.getFileHandle(name, { create: true });
    cleanup = async () => {
      try {
        await directory.removeEntry(name);
      } catch (error) {
        if (!(error instanceof DOMException && error.name === "NotFoundError")) throw error;
      }
    };
    destination = { type: "opfs", handle };
  } else if (target?.type === "file") {
    destination = target;
  }

  let connection: Connection | undefined;
  let completed = false;
  let cancel: (() => void) | undefined;
  try {
    signal?.throwIfAborted();
    const worker = await Connection.start(spawn, new OffscreenCanvas(0, 0));
    connection = worker;
    const cancelled = new Promise<never>((_, reject) => {
      cancel = () => {
        worker.worker.postMessage({ cancelExport: true });
        reject(new PlayerError("cancelled", "the export was cancelled"));
      };
      signal?.addEventListener("abort", cancel, { once: true });
      worker.stopped = reject;
      if (signal?.aborted) cancel();
    });
    const run = async () => {
      const info = (await worker.send({
        type: "open",
        entry: spawn.entry,
        inputs: spawn.inputs,
      })) as Opened;
      const audio = includeAudio && (await worker.send({ type: "sound", entry: spawn.entry }));
      const { width, height } = size(info, options);
      if (
        ![width, height].every((v) => Number.isInteger(v) && v > 0 && v <= 7680) ||
        width * height > 7680 * 4320
      ) {
        throw new PlayerError(
          "scene",
          "output size must be nonzero, at most 7680 px per side and 33177600 pixels (8K)",
        );
      }
      if (width % 2 || height % 2) {
        throw new PlayerError("scene", "H.264 needs even output dimensions");
      }
      const [n, d] =
        options.frameRate === undefined ? rate(info.fps) : rate(String(options.frameRate));
      const frameCount = exportSchedule(info, [n, d]).count;
      const bitrate = options.videoBitrate ?? Math.round(0.15 * width * height * (n / d));
      const mp4 = (await worker.send(
        {
          type: "export",
          destination,
          options: { width, height, rate: [n, d], bitrate, audio: Boolean(audio) },
        },
        [],
        (framesCompleted) => onProgress?.({ framesCompleted, frameCount }),
      )) as ArrayBuffer | undefined;
      if (destination) return { fileHandle: destination.handle, cleanup };
      return { blob: new Blob([mp4!], { type: "video/mp4" }) };
    };
    const result = await Promise.race([run(), cancelled]);
    completed = true;
    return result;
  } finally {
    if (cancel) signal?.removeEventListener("abort", cancel);
    await connection?.close();
    if (!completed) await cleanup?.();
  }
}

/** The frame size `options` ask for: given one side, the other keeps the composition's shape,
 * rounded to an even number, as H.264 wants. */
function size(info: Opened, options: ExportVideoOptions): { width: number; height: number } {
  const { width, height } = options;
  const even = (x: number) => Math.max(2, 2 * Math.round(x / 2));
  if (width === undefined && height === undefined) {
    return { width: info.width, height: info.height };
  }
  if (width === undefined) {
    return { width: even((height! * info.width) / info.height), height: height! };
  }
  if (height === undefined) {
    return { width, height: even((width * info.height) / info.width) };
  }
  return { width, height };
}

/** Frames a second as a fraction: `30`, `29.97` or `30000/1001`. */
function rate(fps: string): [number, number] {
  if (fps.includes("/")) {
    const [n, d] = fps.split("/").map(Number);
    return [n, d];
  }
  const x = Number(fps);
  if (!(x > 0)) {
    throw new PlayerError("scene", `${fps} is not a frame rate`);
  }
  // 29.97 and the like are NTSC's.
  if (!Number.isInteger(x) && Math.abs((Math.round(x) * 1000) / 1001 - x) < 0.01) {
    return [Math.round(x) * 1000, 1001];
  }
  return Number.isInteger(x) ? [x, 1] : [Math.round(x * 1000), 1000];
}
