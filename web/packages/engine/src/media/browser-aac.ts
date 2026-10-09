import { EncodedAudioPacketSource, EncodedPacket } from "mediabunny";

const delays = new Map<string, Promise<number>>();

/** Find the PCM delay of the active AAC encoder instead of assuming a platform-specific priming length. */
async function encoderDelay(config: AudioEncoderConfig): Promise<number> {
  const key = JSON.stringify(config);
  const cached = delays.get(key);
  if (cached) return cached;
  const pending = measureEncoderDelay(config);
  delays.set(key, pending);
  try { return await pending; } catch (error) { delays.delete(key); throw error; }
}

async function measureEncoderDelay(config: AudioEncoderConfig): Promise<number> {
  // WebCodecs currently has no encoder-delay metadata. A short deterministic probe measures the
  // codec round trip; its PCM never enters the work. See w3c/webcodecs#626.
  const prefix = 1024, referenceLength = 1024, length = 8192, maximumDelay = 4096;
  const reference = new Float32Array(referenceLength);
  let seed = 1, previous = 0;
  for (let i = 0; i < reference.length; i++) {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    previous = previous * 0.7 + (seed / 0x1_0000_0000 - 0.5) * 0.3;
    reference[i] = previous;
  }
  const pcm = new Float32Array(length * 2);
  pcm.set(reference, prefix); pcm.set(reference, length + prefix);
  const chunks: EncodedAudioChunk[] = [];
  let decoderConfig: AudioDecoderConfig | undefined;
  let failure: Error | undefined;
  const encoder = new AudioEncoder({ output(chunk, meta) {
    chunks.push(chunk); decoderConfig ??= meta?.decoderConfig;
  }, error(error) { failure = error; } });
  try {
    encoder.configure(config);
    const sample = new AudioData({ format: "f32-planar", sampleRate: config.sampleRate,
      numberOfChannels: 2, numberOfFrames: length, timestamp: 0, data: pcm });
    try { encoder.encode(sample); } finally { sample.close(); }
    await encoder.flush();
    if (failure) throw failure;
  } finally { if (encoder.state !== "closed") encoder.close(); }
  if (!decoderConfig) throw new Error("AAC encoder returned no decoder configuration.");
  const decoded = new Float32Array(length + maximumDelay * 2);
  const decoder = new AudioDecoder({ output(sample) {
    try {
      const values = new Float32Array(sample.numberOfFrames);
      sample.copyTo(values, { planeIndex: 0, format: "f32-planar" });
      const start = Math.round(sample.timestamp * config.sampleRate / 1_000_000);
      const from = Math.max(0, -start), to = Math.min(values.length, decoded.length - start);
      if (to > from) decoded.set(values.subarray(from, to), start + from);
    } finally { sample.close(); }
  }, error(error) { failure = error; } });
  try {
    decoder.configure(decoderConfig);
    for (const chunk of chunks) decoder.decode(chunk);
    await decoder.flush();
    if (failure) throw failure;
  } finally { if (decoder.state !== "closed") decoder.close(); }
  let best = -Infinity, delay = 0;
  const referenceEnergy = reference.reduce((sum, value) => sum + value * value, 0);
  for (let candidate = 0; candidate <= maximumDelay; candidate++) {
    let dot = 0, energy = 0;
    for (let i = 0; i < reference.length; i++) {
      const value = decoded[prefix + candidate + i]!;
      dot += reference[i]! * value; energy += value * value;
    }
    const correlation = dot / Math.sqrt(referenceEnergy * energy);
    if (correlation > best) { best = correlation; delay = candidate; }
  }
  if (best < 0.8 || !Number.isFinite(best)) throw new Error("Could not verify AAC encoder timing in this browser.");
  return delay;
}

/** AAC packets with verified priming and exact authored end time, suitable for an MP4 edit list. */
export async function createBrowserAacEncoder(sampleRate: number, sampleCount: number, bitrate = 192_000) {
  const config: AudioEncoderConfig = { codec: "mp4a.40.2", sampleRate, numberOfChannels: 2, bitrate };
  const delay = await encoderDelay(config);
  const source = new EncodedAudioPacketSource("aac");
  let failure: unknown;
  let writes = Promise.resolve();
  let nextSample = 0;
  const check = () => { if (failure) throw failure; };
  const encoder = new AudioEncoder({ output(chunk, meta) {
    const start = Math.round(chunk.timestamp * sampleRate / 1_000_000) - delay;
    if (start >= sampleCount) return; // Encoder padding after the authored work.
    const frames = Math.round((chunk.duration ?? (1024 / sampleRate * 1_000_000)) * sampleRate / 1_000_000);
    const packet = EncodedPacket.fromEncodedChunk(chunk).clone({ timestamp: start / sampleRate,
      duration: Math.min(frames, sampleCount - start) / sampleRate });
    writes = writes.then(() => source.add(packet, meta)).catch((error) => { failure ??= error; });
  }, error(error) { failure ??= error; } });
  encoder.configure(config);
  return {
    source,
    async add(buffer: AudioBuffer) {
      check();
      if (buffer.sampleRate !== sampleRate || buffer.numberOfChannels !== 2) throw new Error("Export audio format changed.");
      // Keep native encoder queues small and preserve absolute sample timestamps across PCM chunks.
      for (let offset = 0; offset < buffer.length; offset += 16_384) {
        const length = Math.min(16_384, buffer.length - offset);
        const pcm = new Float32Array(length * 2);
        for (let channel = 0; channel < 2; channel++) pcm.set(buffer.getChannelData(channel).subarray(offset, offset + length), channel * length);
        const sample = new AudioData({ format: "f32-planar", sampleRate, numberOfChannels: 2,
          numberOfFrames: length, timestamp: Math.round(nextSample / sampleRate * 1_000_000), data: pcm });
        try { encoder.encode(sample); } finally { sample.close(); }
        nextSample += length;
        if (encoder.encodeQueueSize >= 4) await new Promise<void>((resolve) => encoder.addEventListener("dequeue", () => resolve(), { once: true }));
        check();
        await writes;
      }
    },
    async finish() {
      await encoder.flush(); await writes; check();
      if (nextSample !== sampleCount) throw new Error("Export audio sample count changed.");
      encoder.close(); source.close();
    },
    close() { if (encoder.state !== "closed") encoder.close(); },
  };
}
