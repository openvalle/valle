import { demux, type Mp4InputBuffer } from "./mp4-demux.ts";
import type { DemuxSample as ValleMp4Sample } from "./mp4-demux.ts";
// Browser-resident streaming video decoder.
//
// Per-asset browser frame source using WebCodecs and the bundled MP4Box module.
//
// Demux once, then decode incrementally. Rebase timestamps to the first displayed frame, select
// the latest frame at or before the requested time, clamp before the start, and hold the last
// frame after EOF. Convert chunk timestamps with `cts * 1e6 / timescale`, truncated to integer
// microseconds by WebIDL.
//
// Feed strategy:
//   - sequential forward requests feed chunks incrementally from the decode cursor (never a
//     whole-file re-decode);
//   - a backward request — or a forward jump that can start from a later keyframe — resets the
//     decoder and re-feeds from the nearest keyframe at-or-before the target (in decode order);
//   - the ring retains at most `lookahead` decoded VideoFrames behind/at the playhead; frames
//     that fall out are close()d immediately (zero-copy discipline). Frames ahead of the
//     target are bounded by the feed discipline: feeding stops as soon as the target lands, so
//     at most a reorder-window of future frames ever accumulates.
// `frameAt` resolves to a clone() of the ring's frame: the caller owns it and must close() it;
// the ring keeps (and eventually closes) its own reference.

const FRAME_TIME_EPS_S = 1e-6; // mirror Media codec/decode.rs FRAME_TIME_EPS_S

export interface DecoderRing {
  frameAt(timeS: number): Promise<VideoFrame>;
  baseTimestampUs(): number;
  close(): void;
}

export interface KeyframeThumbnail {
  tS: number;
  bitmap: ImageBitmap;
}

export function createDecoderRing({
  buffer,
  lookahead = 8,
}: {
  buffer: Mp4InputBuffer;
  lookahead?: number;
}): DecoderRing {
  if (!Number.isInteger(lookahead) || lookahead < 1) {
    throw new Error(`lookahead must be a positive integer, got ${lookahead}`);
  }

  let closed = false;
  let decoder: VideoDecoder | null = null;
  let config: VideoDecoderConfig | null = null;
  let samples: ValleMp4Sample[] = []; // decode order (as delivered by mp4box)
  let order: Array<{ decodeIndex: number; tsUs: number }> = [];
  let syncBefore: number[] = [];
  let baseTsUs = 0; // min chunk timestamp == first displayed frame's timestamp
  let cursor = 0; // next decode-order sample index to feed
  let maxEmittedTsUs = -Infinity; // largest frame timestamp emitted since the last reset
  let neededTsUs = -Infinity; // current target timestamp (guards ring trimming)
  let ring: VideoFrame[] = []; // decoded VideoFrames, presentation-ordered, owned by the ring
  let decodeError: Error | null = null;
  let initPromise: Promise<void> | null = null;
  let queue: Promise<unknown> = Promise.resolve(); // serializes frameAt calls

  function onFrame(frame: VideoFrame): void {
    if (closed) {
      frame.close();
      return;
    }
    if (frame.timestamp > maxEmittedTsUs) maxEmittedTsUs = frame.timestamp;
    // Insert outputs in presentation order defensively.
    let at = ring.length;
    while (at > 0 && ring[at - 1].timestamp > frame.timestamp) at -= 1;
    ring.splice(at, 0, frame);
    // Zero-copy discipline: frames at-or-behind the playhead beyond the lookahead window are
    // closed immediately. Never trim the target itself or frames ahead of it.
    while (ring.length > lookahead && ring[0].timestamp < neededTsUs) {
      ring.shift()?.close();
    }
  }

  async function init(): Promise<void> {
    const demuxed = await demux(buffer);
    samples = demuxed.samples;
    if (samples.length === 0) throw new Error("video track has no samples");
    order = samples
      .map((sample, decodeIndex) => ({ decodeIndex, tsUs: chunkTimestampUs(sample) }))
      .sort((a, b) => a.tsUs - b.tsUs);
    baseTsUs = order[0].tsUs;
    syncBefore = new Array(samples.length);
    let lastSync = 0;
    samples.forEach((sample, i) => {
      if (sample.is_sync) lastSync = i;
      syncBefore[i] = lastSync;
    });

    if (typeof VideoDecoder === "undefined") {
      throw new Error("WebCodecs VideoDecoder unavailable");
    }
    const decoderConfig: VideoDecoderConfig = {
      codec: demuxed.track.codec,
      description: demuxed.description,
    };
    config = decoderConfig;
    const support = await VideoDecoder.isConfigSupported(decoderConfig);
    if (!support.supported) {
      throw new Error(`codec ${demuxed.track.codec} unsupported by VideoDecoder`);
    }
    decoder = new VideoDecoder({
      output: onFrame,
      error: (e) => {
        decodeError = decodeError ?? new Error(`decoder: ${e.message}`);
      },
    });
    decoder.configure(decoderConfig);
  }

  function feed(sample: ValleMp4Sample): void {
    requireDecoder().decode(
      new EncodedVideoChunk({
        type: sample.is_sync ? "key" : "delta",
        timestamp: (sample.cts * 1_000_000) / sample.timescale,
        duration: (sample.duration * 1_000_000) / sample.timescale,
        data: sample.data,
      }),
    );
  }

  function seekTo(syncIndex: number): void {
    for (const frame of ring) frame.close();
    ring = [];
    maxEmittedTsUs = -Infinity;
    const active = requireDecoder();
    active.reset(); // drops queued chunks/frames without outputs
    if (!config) throw new Error("decoder config is unavailable");
    active.configure(config);
    cursor = syncIndex;
  }

  function ringFrameAt(tsUs: number): VideoFrame | null {
    return ring.find((frame) => frame.timestamp === tsUs) ?? null;
  }

  async function frameAtInner(tS: number): Promise<VideoFrame> {
    if (closed) throw new Error("decoder ring is closed");
    if (!initPromise) initPromise = init();
    await initPromise;
    if (decodeError) throw decodeError;

    // Target selection over the presentation-ordered samples table: latest pts <= t + eps,
    // clamped to the first frame (t before start) and the last frame (EOF hold).
    let p = 0;
    while (
      p + 1 < order.length &&
      (order[p + 1].tsUs - baseTsUs) / 1_000_000 <= tS + FRAME_TIME_EPS_S
    ) {
      p += 1;
    }
    const target = order[p];
    neededTsUs = target.tsUs;

    // Monotone playback: frames strictly before the new target can never be selected again
    // (a smaller t seeks below); release them eagerly.
    while (ring.length > 0 && ring[0].timestamp < target.tsUs) {
      ring.shift()?.close();
    }
    const hit = ringFrameAt(target.tsUs);
    if (hit) return hit.clone();

    const targetFed = target.decodeIndex < cursor;
    if (targetFed && maxEmittedTsUs >= target.tsUs) {
      // Already decoded and discarded (backward request, or a frame trimmed out of the ring):
      // re-feed from the nearest keyframe at-or-before the target.
      seekTo(syncBefore[target.decodeIndex]);
    } else if (!targetFed && syncBefore[target.decodeIndex] > cursor) {
      // Forward jump past a keyframe: decoding the intermediate GOP tail would be wasted
      // work, so restart from that keyframe instead.
      seekTo(syncBefore[target.decodeIndex]);
    }

    // Incremental feed: burst up to the target's decode index (the decoder queues freely),
    // then chunk-by-chunk with an event-loop yield so output callbacks can land. EOF drains
    // via flush() (mirrors the native EOF-holds-last-frame semantics).
    while (!ringFrameAt(target.tsUs)) {
      if (decodeError) throw decodeError;
      if (closed) throw new Error("decoder ring closed during decode");
      if (cursor < samples.length) {
        feed(samples[cursor]);
        cursor += 1;
        if (cursor > target.decodeIndex || cursor % 16 === 0) await tick();
      } else {
        await requireDecoder().flush();
        if (decodeError) throw decodeError;
        if (!ringFrameAt(target.tsUs)) {
          throw new Error(`no decoded frame for t=${tS} (pts ${(target.tsUs - baseTsUs) / 1e6}s)`);
        }
      }
    }
    const frame = ringFrameAt(target.tsUs);
    if (!frame) throw new Error(`decoded frame ${target.tsUs} disappeared from the ring`);
    return frame.clone();
  }

  return {
    /// Resolve the display frame for source time `tS` (seconds). The returned VideoFrame is
    /// owned by the caller: close() it when done.
    frameAt(tS: number): Promise<VideoFrame> {
      const result = queue.then(() => frameAtInner(tS));
      queue = result.catch(() => {}); // one failed request must not poison the next
      return result;
    },
    /// Timestamp origin (µs) the pts rebase subtracts — the first displayed frame's chunk
    /// timestamp. Valid after the first frameAt() resolves.
    baseTimestampUs(): number {
      if (!initPromise) throw new Error("decoder ring not initialized yet");
      return baseTsUs;
    },
    close(): void {
      if (closed) return;
      closed = true;
      for (const frame of ring) frame.close();
      ring = [];
      if (decoder && decoder.state !== "closed") decoder.close();
    },
  };

  function requireDecoder(): VideoDecoder {
    if (!decoder) throw new Error("decoder ring is not initialized");
    return decoder;
  }
}

// Extract keyframe-only thumbnails for timeline strips.
//
// Decode only sync samples to avoid GOP-prefix work. Process bounded batches, flush, downsample
// each frame immediately, and close it. Return bitmaps in ascending presentation time relative to
// the first displayed frame.
export async function keyframeThumbnails({
  buffer,
  height = 26,
  maxCount = 60,
}: {
  buffer: Mp4InputBuffer;
  height?: number;
  maxCount?: number;
}): Promise<KeyframeThumbnail[]> {
  const demuxed = await demux(buffer);
  const samples = demuxed.samples;
  if (samples.length === 0) throw new Error("video track has no samples");
  let baseTsUs = Infinity;
  for (const s of samples) baseTsUs = Math.min(baseTsUs, chunkTimestampUs(s));
  const keyframes = samples.filter((s) => s.is_sync);
  if (keyframes.length === 0) throw new Error("video track has no keyframes");
  const picked: ValleMp4Sample[] = [];
  const n = Math.min(maxCount, keyframes.length);
  for (let i = 0; i < n; i += 1) {
    picked.push(keyframes[n === 1 ? 0 : Math.round((i * (keyframes.length - 1)) / (n - 1))]);
  }

  if (typeof VideoDecoder === "undefined") throw new Error("WebCodecs VideoDecoder unavailable");
  const config: VideoDecoderConfig = {
    codec: demuxed.track.codec,
    description: demuxed.description,
  };
  const support = await VideoDecoder.isConfigSupported(config);
  if (!support.supported) throw new Error(`codec ${demuxed.track.codec} unsupported by VideoDecoder`);

  const out: KeyframeThumbnail[] = [];
  let decodeError: Error | null = null;
  let bitmapJobs: Array<Promise<void>> = [];
  const decoder = new VideoDecoder({
    output: (frame) => {
      const tS = (frame.timestamp - baseTsUs) / 1_000_000;
      const scale = height / Math.max(1, frame.displayHeight);
      const w = Math.max(1, Math.round(frame.displayWidth * scale));
      bitmapJobs.push(
        createImageBitmap(frame, { resizeWidth: w, resizeHeight: height })
          .then((bitmap) => { out.push({ tS, bitmap }); })
          .finally(() => frame.close()),
      );
    },
    error: (e) => {
      decodeError = decodeError ?? new Error(`thumbnail decoder: ${e.message}`);
    },
  });
  try {
    decoder.configure(config);
    const BATCH = 8;
    for (let i = 0; i < picked.length; i += BATCH) {
      for (const s of picked.slice(i, i + BATCH)) {
        decoder.decode(
          new EncodedVideoChunk({
            type: "key",
            timestamp: (s.cts * 1_000_000) / s.timescale,
            duration: (s.duration * 1_000_000) / s.timescale,
            data: s.data,
          }),
        );
      }
      await decoder.flush();
      await Promise.all(bitmapJobs);
      bitmapJobs = [];
      if (decodeError) throw decodeError;
    }
  } finally {
    if (decoder.state !== "closed") decoder.close();
  }
  out.sort((a, b) => a.tS - b.tS);
  return out;
}

function tick(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

// Chunk timestamps go through WebIDL `long long` conversion (truncation toward zero); compute
// the stored integer the same way so ring lookups match decoded frame timestamps exactly.
function chunkTimestampUs(sample: ValleMp4Sample): number {
  return Math.trunc((sample.cts * 1_000_000) / sample.timescale);
}
