import { expect, test } from "bun:test";
import { createFile } from "mp4box";
import { demux } from "./mp4-demux.ts";

test("extracts all batches and preserves encoded bytes, decode times and presentation order", async () => {
  const file = createFile();
  // Minimal AVC configuration: no SPS/PPS payloads needed for this demux-only fixture.
  const description = new Uint8Array([1, 66, 0, 30, 255, 224, 0]);
  const track = file.addTrack({
    type: "avc1", width: 16, height: 16, timescale: 30,
    avcDecoderConfigRecord: description.buffer,
  });
  for (let i = 0; i < 1005; i++) {
    file.addSample(track, new Uint8Array([i % 256, 7]), {
      duration: 1, dts: i, cts: 3 + (i === 1 ? 2 : i === 2 ? 1 : i), is_sync: i % 30 === 0,
    });
  }
  const result = await demux(file.getBuffer().buffer);
  expect(result.track.timescale).toBe(30);
  expect(result.description).toEqual(description);
  expect(result.samples).toHaveLength(1005);
  expect(result.samples[1004]!.data).toEqual(new Uint8Array([1004 % 256, 7]));
  expect(result.samples.slice(0, 3).map((sample) => sample.dts)).toEqual([0, 1, 2]);
  expect(result.samples.slice(0, 3).map((sample) => sample.cts)).toEqual([3, 5, 4]);
  expect(result.samples[0]!.is_sync).toBe(true);
  expect(result.samples[1]!.is_sync).toBe(false);
});

test("rejects input without a video track", async () => {
  await expect(demux(new ArrayBuffer(0))).rejects.toThrow("no video track");
});
