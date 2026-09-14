import { createFile, DataStream, Endianness, Log, type MP4BoxBuffer, type Sample, type Track } from "mp4box";

export type Mp4InputBuffer = ArrayBuffer & { fileStart?: number };
export type DemuxSample = Sample & { data: Uint8Array<ArrayBuffer> };
export interface DemuxResult {
  track: Track;
  samples: DemuxSample[];
  description?: Uint8Array<ArrayBuffer>;
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
