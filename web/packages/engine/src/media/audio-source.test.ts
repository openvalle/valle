import { expect, test } from "bun:test";
import { BrowserAudioWindowReader } from "./audio-source.ts";

test("random reads from a long WAV allocate only requested PCM windows and preserve samples", async () => {
  const rate = 48000,
    seconds = 30,
    channels = 2;
  const encoded = new Uint8Array(44 + rate * seconds * channels * 2),
    data = new DataView(encoded.buffer);
  const text = (offset: number, value: string) =>
    encoded.set(new TextEncoder().encode(value), offset);
  text(0, "RIFF");
  data.setUint32(4, encoded.length - 8, true);
  text(8, "WAVEfmt ");
  data.setUint32(16, 16, true);
  data.setUint16(20, 1, true);
  data.setUint16(22, channels, true);
  data.setUint32(24, rate, true);
  data.setUint32(28, rate * 4, true);
  data.setUint16(32, 4, true);
  data.setUint16(34, 16, true);
  text(36, "data");
  data.setUint32(40, encoded.length - 44, true);
  const value = (sample: number, channel: number) =>
    ((sample * (channel ? 17 : 13)) % 60000) - 30000;
  for (let sample = 0; sample < rate * seconds; sample++)
    for (let channel = 0; channel < channels; channel++)
      data.setInt16(44 + (sample * 2 + channel) * 2, value(sample, channel), true);
  let maximum = 0;
  const context = {
    sampleRate: rate,
    createBuffer(count: number, length: number, sampleRate: number) {
      maximum = Math.max(maximum, length);
      const planes = Array.from({ length: count }, () => new Float32Array(length));
      return {
        sampleRate,
        numberOfChannels: count,
        length,
        getChannelData: (channel: number) => planes[channel]!,
      };
    },
  } as unknown as BaseAudioContext;
  const reader = await BrowserAudioWindowReader.open({ buffer: encoded });
  try {
    for (const second of [20, 0, 7, 29, 7]) {
      const start = second * rate,
        buffer = await reader.read(context, start, start + rate);
      expect(buffer.length).toBe(rate);
      for (const sample of [0, 1, 1599, 1600, 32767, rate - 1])
        for (let channel = 0; channel < 2; channel++) {
          expect(buffer.getChannelData(channel)[sample]).toBe(
            value(start + sample, channel) / 32768,
          );
        }
    }
    expect(maximum).toBe(rate);
    await expect(reader.read(context, 0, rate * 2)).rejects.toThrow("window");
  } finally {
    reader.close();
  }
});
