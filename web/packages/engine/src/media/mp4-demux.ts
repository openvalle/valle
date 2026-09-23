import { createFile, DataStream, Endianness, Log, type MP4BoxBuffer, type Sample, type Track } from "mp4box";

export type Mp4InputBuffer = ArrayBuffer & { fileStart?: number };
export type DemuxSample = Sample & { data: Uint8Array<ArrayBuffer> };
export interface DemuxResult {
  track: Track;
  samples: DemuxSample[];
  description?: Uint8Array<ArrayBuffer>;
  readSample?: (sample: DemuxSample) => Promise<Uint8Array<ArrayBuffer>>;
}

/** Admit a seekable, host-provided Blob without retaining mdat in the JS heap. */
export async function demuxBlob(blob: Blob): Promise<DemuxResult> {
  const file = createFile(false);
  let track: Track | undefined;
  let error: Error | undefined;
  file.onError = (module, message) => { error = new Error(`mp4box ${module}: ${message}`); };
  file.onReady = (info) => { track = info.videoTracks[0]; };
  // MP4Box returns the next required offset, skipping mdat when the moov is at the end.
  let offset = 0;
  while (offset < blob.size) {
    const buffer = await blob.slice(offset, offset + 1024 * 1024).arrayBuffer() as MP4BoxBuffer;
    buffer.fileStart = offset;
    const next = file.appendBuffer(buffer, offset + buffer.byteLength >= blob.size);
    if (error) throw error;
    offset = next > offset ? next : offset + buffer.byteLength;
  }
  if (!track) throw new Error("no video track");
  const samples = file.getTrackSamplesInfo(track.id).map((sample) => ({
    ...sample, data: new Uint8Array(0),
  })) as DemuxSample[];
  if (!samples.length) throw new Error("video track has no samples");
  const entries = file.getTrackById(track.id).mdia.minf.stbl.stsd.entries;
  const codecBoxes = new Set(["avcC", "hvcC", "vpcC", "av1C"]);
  let description: Uint8Array<ArrayBuffer> | undefined;
  for (const entry of entries) {
    const box = entry.boxes?.find((candidate) => codecBoxes.has(candidate.type));
    if (!box) continue;
    const stream = new DataStream(undefined, 0, Endianness.BIG_ENDIAN);
    box.write(stream);
    description = new Uint8Array(stream.buffer.slice(8));
    break;
  }
  // Read one small window for sequential playback; seeking replaces it.
  let windowStart = -1, window = new Uint8Array(0);
  return { track, samples, description, async readSample(sample) {
    const start = sample.offset, end = start + sample.size;
    if (!Number.isSafeInteger(start) || start < 0 || end > blob.size) throw new Error("MP4 sample exceeds source file");
    if (start < windowStart || end > windowStart + window.length) {
      windowStart = start;
      window = new Uint8Array(await blob.slice(start, Math.max(end, start + 1024 * 1024)).arrayBuffer());
    }
    return window.subarray(start - windowStart, end - windowStart);
  } };
}

/** Extract encoded samples once; WebCodecs decoding and playback stay in decoder-ring. */
export async function demux(buffer: Mp4InputBuffer): Promise<DemuxResult> {
  Log.setLogLevel(Log.error);
  const file = createFile();
  const state: { track?: Track; error?: Error } = {};
  const samples: DemuxSample[] = [];
  file.onError = (module, message) => { state.error = new Error(`mp4box ${module}: ${message}`); };
  file.onReady = (info) => {
    state.track = info.videoTracks[0];
    if (!state.track) return;
    file.setExtractionOptions(state.track.id, null, { nbSamples: 1000 });
    file.start();
  };
  file.onSamples = (_id, _user, chunk) => {
    for (const sample of chunk) {
      if (!sample.data) { state.error = new Error("MP4 sample has no encoded data"); return; }
      samples.push({ ...sample, data: sample.data });
    }
  };
  buffer.fileStart = 0;
  file.appendBuffer(buffer as MP4BoxBuffer);
  file.flush();
  if (state.error) throw state.error;
  if (!state.track) throw new Error("no video track");
  if (samples.length === 0) throw new Error("video track has no samples");

  const entries = file.getTrackById(state.track.id).mdia.minf.stbl.stsd.entries;
  const codecBoxes = new Set(["avcC", "hvcC", "vpcC", "av1C"]);
  let description: Uint8Array<ArrayBuffer> | undefined;
  for (const entry of entries) {
    const box = entry.boxes?.find((candidate) => codecBoxes.has(candidate.type));
    if (!box) continue;
    const stream = new DataStream(undefined, 0, Endianness.BIG_ENDIAN);
    box.write(stream);
    description = new Uint8Array(stream.buffer.slice(8));
    break;
  }
  return { track: state.track, samples, description };
}
