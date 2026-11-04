// Exporting in the browser: what a player has open, as it is bound now, rendered again by a
// worker of its own into an MP4, so playing, seeking and the volume do not touch it.
//
//   import { exportBrowserVideo } from "valle-engine/export";
//   const { blob } = await exportBrowserVideo(player, { width: 1280, height: 720 });

import { Connection, PlayerError, spawns } from "./connection.ts";
import type { Player } from "./index.ts";
import { exportFrames, type Opened } from "./messages.ts";

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

/** The MP4 of what `player` has open: H.264, and its sound, AAC where the browser encodes it,
 * else Opus. */
export async function exportBrowserVideo(
  player: Player,
  options: ExportVideoOptions = {},
): Promise<{ blob: Blob }> {
  const spawn = spawns.get(player);
  if (!spawn) {
    throw new PlayerError("scene", "the player was closed");
  }
  const { signal, onProgress, includeAudio = true } = options;
  signal?.throwIfAborted();
  const connection = await Connection.start(spawn, new OffscreenCanvas(0, 0));
  const cancelled = new Promise<never>((_, reject) => {
    const cancel = () => reject(new PlayerError("cancelled", "the export was cancelled"));
    signal?.addEventListener("abort", cancel, { once: true });
    connection.stopped = reject;
  });
  try {
    const run = async () => {
      const info = (await connection.send({
        type: "open",
        entry: spawn.entry,
        inputs: spawn.inputs,
      })) as Opened;
      const audio = includeAudio && (await connection.send({ type: "sound", entry: spawn.entry }));
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
      const frameCount = exportFrames(info, [n, d]).length;
      const bitrate = options.videoBitrate ?? Math.round(0.15 * width * height * (n / d));
      const mp4 = (await connection.send(
        {
          type: "export",
          options: { width, height, rate: [n, d], bitrate, audio: Boolean(audio) },
        },
        [],
        (framesCompleted) => onProgress?.({ framesCompleted, frameCount }),
      )) as ArrayBuffer;
      return { blob: new Blob([mp4], { type: "video/mp4" }) };
    };
    return await Promise.race([run(), cancelled]);
  } finally {
    await connection.close();
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
