import { EncodedPacketSink, VideoSample, VideoSampleSink, type InputVideoTrack } from "mediabunny";
import { createMediaInput, type BrowserMediaInput } from "./input.ts";

const FRAME_TIME_EPS_S = 1e-6; // Match the native frame-at-source-time boundary.
let proresDecoder: Promise<void> | undefined;

async function prepareDecoder(track: InputVideoTrack): Promise<void> {
  if (track.codec === "prores") {
    await (proresDecoder ??= import("@mediabunny/prores").then(({ registerProresDecoder }) => {
      registerProresDecoder();
    }));
  }
  if (!(await track.canDecode())) {
    throw new Error(
      `This browser cannot decode video codec '${track.codec ?? track.internalCodecId}'.`,
    );
  }
}

export interface DecoderRing {
  frameAt(timeS: number): Promise<VideoFrame>;
  baseTimestampUs(): number;
  close(): void;
}

export interface KeyframeThumbnail {
  tS: number;
  bitmap: ImageBitmap;
}

/**
 * Thin adapter for the existing frame source API. Mediabunny owns demuxing, decoder queues,
 * keyframe seeks and frame lifetimes. Only source-time rebasing and EOF hold belong here.
 */
export function createDecoderRing({
  lookahead = 8,
  ...source
}: BrowserMediaInput & { lookahead?: number }): DecoderRing {
  if (!Number.isInteger(lookahead) || lookahead < 1)
    throw new Error(`lookahead must be a positive integer, got ${lookahead}`);
  const input = createMediaInput(source);
  let closed = false;
  let baseTime: number | undefined;
  let sink: VideoSampleSink;
  let iterator: AsyncGenerator<VideoSample> | undefined;
  let current: VideoSample | undefined, next: VideoSample | undefined;
  let initialized: Promise<void> | undefined;
  let proresColor: VideoColorSpaceInit | undefined;
  let queue: Promise<unknown> = Promise.resolve();
  const checkOpen = () => {
    if (closed) throw new Error("video source is closed");
  };
  const initialize = async () => {
    const track = await input.getPrimaryVideoTrack();
    if (!track) throw new Error("no video track");
    await prepareDecoder(track);
    if (track.codec === "prores") proresColor = await track.getColorSpace();
    baseTime = await track.getFirstTimestamp();
    checkOpen();
    sink = new VideoSampleSink(track);
  };
  const stopIteration = () => {
    void iterator?.return(undefined).catch(() => undefined);
    iterator = undefined;
    current?.close();
    next?.close();
    current = undefined;
    next = undefined;
  };
  const take = async (active: AsyncGenerator<VideoSample>) => {
    let sample = (await active.next()).value || undefined;
    if (sample && proresColor) sample = await restoreProresColor(sample, proresColor);
    if (closed) {
      sample?.close();
      checkOpen();
    }
    return sample;
  };
  async function frameAt(timeS: number): Promise<VideoFrame> {
    checkOpen();
    if (Number.isNaN(timeS)) throw new Error("source time must not be NaN");
    await (initialized ??= initialize());
    checkOpen();
    const target = baseTime! + Math.max(0, timeS) + FRAME_TIME_EPS_S;
    // Keep one continuous iterator for playback/export. Restart it for backward seeks or large
    // jumps; the library seeks to the right keyframe without walking every intermediate frame.
    if (
      !iterator ||
      (current && target < current.timestamp) ||
      (current &&
        next &&
        target > next.timestamp + (next.timestamp - current.timestamp) * lookahead)
    ) {
      stopIteration();
      const active = sink.samples(target);
      iterator = active;
      current = await take(active);
      next = await take(active);
    }
    while (next && next.timestamp <= target) {
      current?.close();
      current = next;
      next = await take(iterator!);
    }
    checkOpen();
    if (!current) throw new Error(`no decoded frame for t=${timeS}`);
    // The returned frame is independently owned by the caller, even after the source closes.
    return current.toVideoFrame();
  }
  return {
    frameAt(timeS) {
      const result = queue.then(() => frameAt(timeS));
      queue = result.catch(() => undefined);
      return result;
    },
    baseTimestampUs() {
      if (baseTime === undefined) throw new Error("video source is not initialized");
      return Math.round(baseTime * 1_000_000);
    },
    close() {
      if (closed) return;
      closed = true;
      stopIteration();
      input.dispose();
    },
  };
}

/** The ProRes extension reads frame-header tags, which may be absent even when MOV has colr.
 * Fill only missing YUV tags before the browser converts pixels. Keep the decoded bit depth,
 * alpha, crop and timing; RGB conversion here would already have used the wrong matrix.
 */
async function restoreProresColor(
  sample: VideoSample,
  fallback: VideoColorSpaceInit,
): Promise<VideoSample> {
  const frame = sample.toVideoFrame();
  try {
    const current = frame.colorSpace.toJSON();
    const colorSpace = {
      primaries: current.primaries ?? fallback.primaries,
      transfer: current.transfer ?? fallback.transfer,
      matrix: current.matrix ?? fallback.matrix,
      fullRange: current.fullRange ?? fallback.fullRange,
    };
    if (
      !frame.format?.startsWith("I") ||
      !(["primaries", "transfer", "matrix", "fullRange"] as const).some(
        (field) => current[field] == null && fallback[field] != null,
      )
    )
      return sample;
    const options = { rect: { x: 0, y: 0, width: frame.codedWidth, height: frame.codedHeight } };
    const bytes = new Uint8Array(frame.allocationSize(options));
    const layout = await frame.copyTo(bytes, options);
    const corrected = new VideoFrame(bytes, {
      format: frame.format,
      codedWidth: frame.codedWidth,
      codedHeight: frame.codedHeight,
      visibleRect: frame.visibleRect ?? undefined,
      displayWidth: frame.displayWidth,
      displayHeight: frame.displayHeight,
      timestamp: frame.timestamp,
      duration: frame.duration ?? undefined,
      layout,
      colorSpace,
    });
    const result = new VideoSample(corrected, {
      timestamp: sample.timestamp,
      duration: sample.duration,
    });
    sample.close();
    return result;
  } catch (error) {
    sample.close();
    throw error;
  } finally {
    frame.close();
  }
}

/** Read keyframe metadata and let the same library decode the selected thumbnail frames. */
export async function keyframeThumbnails({
  height = 26,
  maxCount = 60,
  ...source
}: BrowserMediaInput & { height?: number; maxCount?: number }): Promise<KeyframeThumbnail[]> {
  if (!Number.isInteger(height) || height < 1 || !Number.isInteger(maxCount) || maxCount < 1) {
    throw new Error("thumbnail height and maxCount must be positive integers");
  }
  const input = createMediaInput(source);
  const thumbnails: KeyframeThumbnail[] = [];
  try {
    const track = await input.getPrimaryVideoTrack();
    if (!track) throw new Error("no video track");
    await prepareDecoder(track);
    const origin = await track.getFirstTimestamp();
    const packets = new EncodedPacketSink(track);
    const times: number[] = [];
    for (
      let packet = await packets.getFirstKeyPacket({ metadataOnly: true });
      packet;
      packet = await packets.getNextKeyPacket(packet, { metadataOnly: true })
    )
      times.push(packet.timestamp);
    if (!times.length) throw new Error("video track has no keyframes");
    const count = Math.min(maxCount, times.length);
    const selected = Array.from(
      { length: count },
      (_, i) => times[count === 1 ? 0 : Math.round((i * (times.length - 1)) / (count - 1))]!,
    );
    for await (const sample of new VideoSampleSink(track).samplesAtTimestamps(selected)) {
      if (!sample) throw new Error("thumbnail frame could not be decoded");
      const frame = sample.toVideoFrame();
      try {
        const width = Math.max(
          1,
          Math.round((frame.displayWidth * height) / Math.max(1, frame.displayHeight)),
        );
        const bitmap = await createImageBitmap(frame, { resizeWidth: width, resizeHeight: height });
        thumbnails.push({ tS: sample.timestamp - origin, bitmap });
      } finally {
        frame.close();
        sample.close();
      }
    }
    return thumbnails;
  } catch (error) {
    for (const thumbnail of thumbnails) thumbnail.bitmap.close();
    throw error;
  } finally {
    input.dispose();
  }
}
