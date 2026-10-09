import {
  BufferTarget,
  CanvasSource,
  Mp4OutputFormat,
  Output,
  Quality,
  StreamTarget,
  canEncodeAudio,
  canEncodeVideo,
} from "mediabunny";
import {
  BrowserValleWebPlayer,
  type BrowserValleWebPlayerOptions,
} from "./runtime/product-controller.ts";
import { createExportFileSink } from "./media/export-file-sink.ts";
import { createBrowserAacEncoder } from "./media/browser-aac.ts";
import { resolveExportPlan, type BrowserVideoExportSettings } from "./export-settings.ts";

export type { BrowserVideoExportSettings } from "./export-settings.ts";

export interface BrowserVideoExportProgress {
  phase: "loading" | "rendering" | "finalizing";
  framesCompleted: number;
  frameCount: number;
}

export interface BrowserVideoExportOptions extends BrowserVideoExportSettings {
  /** Stream to a user-selected file. Omit to return an MP4 Blob. Owned and closed by the exporter. */
  writable?: FileSystemWritableFileStream;
  signal?: AbortSignal;
  onProgress?: (progress: BrowserVideoExportProgress) => void;
}

export interface BrowserVideoExportResult {
  /** Null when the MP4 was written directly to writable. */
  blob: Blob | null;
  renderId: string;
  width: number;
  height: number;
  frameCount: number;
  frameRate: string;
  durationS: number;
  hasAudio: boolean;
}

/**
 * Render a frozen package into H.264/AAC MP4 entirely in the browser. An isolated player keeps
 * preview seeks, mute/volume, resolution, and subsequent edits out of the exported work.
 * No native render/export endpoint or FFmpeg module participates in this path.
 */
export async function exportBrowserVideo(
  input: BrowserValleWebPlayerOptions,
  options: BrowserVideoExportOptions = {},
): Promise<BrowserVideoExportResult> {
  let player: BrowserValleWebPlayer | undefined;
  let output: Output | undefined;
  let fileSink: ReturnType<typeof createExportFileSink> | undefined;
  let audio: Awaited<ReturnType<typeof createBrowserAacEncoder>> | null = null;
  const checkCancelled = () => options.signal?.throwIfAborted();
  try {
    checkCancelled();
    if (typeof VideoEncoder === "undefined") {
      throw new Error("This browser cannot export MP4. Open Studio in a browser with WebCodecs video encoding support.");
    }
    options.onProgress?.({ phase: "loading", framesCompleted: 0, frameCount: 0 });
    const canvas = document.createElement("canvas");
    player = new BrowserValleWebPlayer({
      ...input,
      assets: input.assets?.map((asset) => ({ ...asset })),
      canvas,
      pxScale: 1,
      audioContext: null,
    });
    await player.init();
    checkCancelled();
    const info = player.renderInfo;
    const plan = resolveExportPlan(info, options);
    const { width, height } = plan;
    const hasAudio = options.includeAudio !== false && player.hasAudio();
    const quality = options.videoBitrate === undefined ? new Quality("high") : new Quality({ bitrate: options.videoBitrate });
    const [videoSupported, audioSupported] = await Promise.all([
      canEncodeVideo("avc", { width, height, quality, frameRate: plan.fps }),
      hasAudio ? canEncodeAudio("aac", { sampleRate: info.sampleRate, numberOfChannels: 2, quality: new Quality({ bitrate: plan.audioBitrate }) }) : true,
    ]);
    checkCancelled();
    if (!videoSupported) throw new Error(`This browser cannot encode H.264 at ${width} × ${height}.`);
    if (!audioSupported) throw new Error("This browser cannot encode AAC audio. Use a browser with AAC encoding support to export this work.");
    if (hasAudio && typeof AudioDecoder === "undefined") throw new Error("This browser cannot verify AAC audio timing. Use a browser with WebCodecs audio decoding support.");

    // Write media incrementally and append the index at the end. A regular MP4 edit list is needed
    // to remove AAC priming while preserving packets required by the decoder.
    fileSink = options.writable ? createExportFileSink(options.writable) : undefined;
    const target = fileSink
      ? new StreamTarget(fileSink.stream, { chunked: true, chunkSize: 1_048_576 })
      : new BufferTarget();
    output = new Output({ format: new Mp4OutputFormat({ fastStart: false }), target });
    const video = new CanvasSource(canvas, { codec: "avc", quality, latencyMode: "quality",
      ...(plan.resize ? { transform: { width, height, fit: "contain" as const } } : {}) });
    // A frame-rate hint snaps every packet duration to a whole frame. Omit it when the final
    // frame is partial so MP4 retains the authored end instead of losing the remaining samples.
    output.addVideoTrack(video, plan.partialFinalFrame ? {} : { frameRate: plan.fps });
    audio = hasAudio ? await createBrowserAacEncoder(info.sampleRate, info.sampleCount, plan.audioBitrate) : null;
    checkCancelled();
    if (audio) output.addAudioTrack(audio.source);
    await output.start();
    let audioSample = 0;
    const addAudioUntil = async (endS: number) => {
      while (audio && audioSample < info.sampleCount && audioSample / info.sampleRate < endS) {
        checkCancelled();
        const endSample = Math.min(info.sampleCount, audioSample + info.sampleRate);
        // Bound offline PCM to one second and interleave tracks to bound the muxer's queue too.
        const buffer = await player!.renderAudioOffline(audioSample / info.sampleRate, endSample / info.sampleRate);
        checkCancelled();
        if (!buffer || buffer.length !== endSample - audioSample) {
          throw new Error("Offline audio does not match the admitted sample schedule.");
        }
        await audio.add(buffer);
        audioSample = endSample;
      }
    };
    for (let frame = 0; frame < plan.frameCount; frame++) {
      checkCancelled();
      const sourceFrame = plan.sourceFrame(frame);
      const result = await player.renderFrame(sourceFrame, { rasterScale: plan.rasterScale });
      if (result.frame.renderId !== info.renderId || result.frame.index !== sourceFrame) {
        throw new Error("Export frame does not match the frozen render package.");
      }
      checkCancelled();
      // Derive every timestamp from the absolute frame number, never an accumulated float clock.
      const timestamp = plan.timestamp(frame);
      const nextTimestamp = plan.endTimestamp(frame);
      await video.add(timestamp, nextTimestamp - timestamp);
      await addAudioUntil(nextTimestamp);
      options.onProgress?.({ phase: "rendering", framesCompleted: frame + 1, frameCount: plan.frameCount });
      // Let progress, cancellation, and the rest of Studio run even with fully cached fast frames.
      if (frame % 8 === 0) await new Promise<void>((resolve) => setTimeout(resolve, 0));
    }
    await addAudioUntil(info.sampleCount / info.sampleRate);
    video.close();
    await audio?.finish();
    checkCancelled();
    options.onProgress?.({ phase: "finalizing", framesCompleted: plan.frameCount, frameCount: plan.frameCount });
    await output.finalize();
    checkCancelled();
    await fileSink?.commit();
    return {
      blob: target instanceof BufferTarget ? new Blob([target.buffer!], { type: "video/mp4" }) : null,
      renderId: info.renderId,
      width,
      height,
      frameCount: plan.frameCount,
      frameRate: plan.frameRate,
      durationS: plan.durationS,
      hasAudio,
    };
  } catch (error) {
    audio?.close();
    if (output) await output.cancel().catch(() => undefined);
    if (fileSink) await fileSink.abort(error).catch(() => undefined);
    else await options.writable?.abort(error).catch(() => undefined);
    throw error;
  } finally {
    audio?.close();
    await player?.close();
  }
}
