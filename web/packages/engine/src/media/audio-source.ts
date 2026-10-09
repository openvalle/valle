import { AudioSampleSink } from "mediabunny";
import { createMediaInput, type BrowserMediaInput } from "./input.ts";

/** Decode with the shared reader. WebAudio remains responsible only for PCM resampling/mixing. */
export async function decodeAudioBuffer(source: BrowserMediaInput, context: BaseAudioContext): Promise<AudioBuffer> {
  const input = createMediaInput(source);
  let buffer: AudioBuffer;
  try {
    const track = await input.getPrimaryAudioTrack();
    if (!track) throw new Error("no audio track");
    const sampleRate = await track.getSampleRate();
    const channels = await track.getNumberOfChannels();
    const length = Math.round(await track.computeDuration() * sampleRate);
    if (!Number.isSafeInteger(length) || length < 1) throw new Error("audio track has no playable samples");
    buffer = context.createBuffer(channels, length, sampleRate);
    for await (const sample of new AudioSampleSink(track).samples(0)) {
      try {
        if (sample.sampleRate !== sampleRate || sample.numberOfChannels !== channels) throw new Error("decoded audio format changed");
        const start = Math.round(sample.timestamp * sampleRate);
        const offset = Math.max(0, -start);
        const count = Math.min(sample.numberOfFrames - offset, length - Math.max(0, start));
        if (count > 0) for (let channel = 0; channel < channels; channel++) {
          sample.copyTo(buffer.getChannelData(channel).subarray(Math.max(0, start), Math.max(0, start) + count), {
            planeIndex: channel, format: "f32-planar", frameOffset: offset, frameCount: count,
          });
        }
      } finally { sample.close(); }
    }
  } finally { input.dispose(); }
  if (buffer.sampleRate === context.sampleRate) return buffer;
  const offline = new OfflineAudioContext(buffer.numberOfChannels,
    Math.max(1, Math.round(buffer.length * context.sampleRate / buffer.sampleRate)), context.sampleRate);
  const node = offline.createBufferSource();
  node.buffer = buffer; node.connect(offline.destination); node.start();
  return offline.startRendering();
}
