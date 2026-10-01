import type { MotionCompileOptions } from "valle-engine";

export interface AudioAnalysisInput {
  control: string;
  contentHash: string;
  url: string;
}
type Pcm = { sampleRate: number; samples: Float32Array };
const decoded = new Map<string, Promise<Pcm>>();

async function decode(input: AudioAnalysisInput): Promise<Pcm> {
  const response = await fetch(`${input.url}/analysis-pcm`);
  if (!response.ok)
    throw new Error(`Audio asset ${input.control}: HTTP ${response.status}`);
  if (response.headers.get("X-Valle-Audio-Source") !== input.contentHash)
    throw new Error(`Audio asset ${input.control} changed after preparation`);
  const sampleRate = Number(response.headers.get("X-Valle-Sample-Rate"));
  const bytes = await response.arrayBuffer();
  if (sampleRate !== 48_000 || !bytes.byteLength || bytes.byteLength % 4)
    throw new Error(`Audio asset ${input.control} has invalid canonical PCM`);
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  const hash = `sha256:${Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
  if (hash !== response.headers.get("X-Valle-Pcm-Digest"))
    throw new Error(`Audio asset ${input.control} PCM changed during transfer`);
  // Host uses FFmpeg's exact CLI downmix/resampler. Browser codecs are never
  // involved, so MP3/AAC delay trimming cannot move preview events by a frame.
  let samples = new Float32Array(bytes);
  if (new Uint8Array(new Uint16Array([1]).buffer)[0] !== 1) {
    const view = new DataView(bytes);
    samples = Float32Array.from({ length: bytes.byteLength / 4 }, (_, i) => view.getFloat32(i * 4, true));
  }
  return { sampleRate, samples };
}

/** Called inside the compiler Worker; canonical PCM is downloaded once per asset. */
export async function prepareAudioAnalysisInputs(
  inputs: readonly AudioAnalysisInput[],
): Promise<NonNullable<MotionCompileOptions["audioSources"]>> {
  return Promise.all(
    inputs.map(async (input) => {
      const key = input.contentHash;
      let pcm = decoded.get(key);
      if (!pcm) {
        pcm = decode(input);
        decoded.set(key, pcm);
        pcm.catch(() => {
          if (decoded.get(key) === pcm) decoded.delete(key);
        });
        if (decoded.size > 4) decoded.delete(decoded.keys().next().value!);
      }
      return {
        control: input.control,
        contentHash: input.contentHash,
        ...(await pcm),
      };
    }),
  );
}

/** The digest identifies PCM; never stringify millions of samples for cache keys. */
export function motionCompileOptionIdentity(
  options?: MotionCompileOptions,
): unknown {
  if (!options) return options;
  return {
    ...options,
    audioSources: options.audioSources?.map(
      ({ samples: _samples, ...metadata }) => metadata,
    ),
  };
}
