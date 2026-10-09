// Runs in a real browser opened by the acceptance host, using the production export entry.
import { exportBrowserVideo, type BrowserVideoExportResult } from "../packages/engine/src/export.ts";
import type { BrowserValleWebPlayerOptions } from "../packages/engine/src/index.ts";
import { createDecoderRing, keyframeThumbnails } from "../packages/engine/src/media/video-source.ts";
import { decodeAudioBuffer } from "../packages/engine/src/media/audio-source.ts";
import { BrowserValleWebPlayer } from "../packages/engine/src/runtime/product-controller.ts";

const status = document.getElementById("status")!;
const report: Array<Record<string, unknown>> = [];
const show = () => { status.textContent = JSON.stringify(report, null, 2); };
function assert(value: unknown, message: string): asserts value { if (!value) throw new Error(message); }
async function input(fixture: string): Promise<BrowserValleWebPlayerOptions> {
  return (await fetch(`/__test/input.json?fixture=${fixture}`)).json();
}
async function save(name: string, result: BrowserVideoExportResult) {
  assert(result.blob?.size, "MP4 Blob is empty");
  const url = URL.createObjectURL(result.blob);
  const video = document.createElement("video");
  try {
    const loaded = new Promise<void>((resolve, reject) => {
      video.onloadedmetadata = () => resolve(); video.onerror = () => reject(new Error("MP4 cannot be played"));
    });
    video.src = url;
    await loaded;
    assert(video.videoWidth === result.width && video.videoHeight === result.height, "decoded video dimensions changed");
    assert(Math.abs(video.duration - result.durationS) < 1 / 48_000, `MP4 duration ${video.duration} differs from work ${result.durationS}`);
  } finally { video.removeAttribute("src"); video.load(); URL.revokeObjectURL(url); }
  const response = await fetch(`/__test/artifacts/${name}.mp4`, { method: "POST", body: result.blob });
  assert(response.ok, "failed to save browser artifact");
}
async function checkMediaReaders() {
  const blob = await (await fetch("/fixtures/offset-bframes.mp4")).blob();
  const source = createDecoderRing({ blob });
  const originalDecode = VideoDecoder.prototype.decode;
  let decodes = 0;
  VideoDecoder.prototype.decode = function (chunk) { decodes++; originalDecode.call(this, chunk); };
  const canvas = new OffscreenCanvas(1, 1), context = canvas.getContext("2d")!;
  const checkFrame = async (time: number) => {
    const frame = await source.frameAt(time);
    try {
      assert(source.baseTimestampUs() === 3_000_000, "nonzero video origin was lost");
      const expectedTime = Number.isFinite(time) ? Math.min(89, Math.max(0, Math.floor(time * 30 + 1e-5))) / 30 : 89 / 30;
      assert(Math.abs((frame.timestamp - source.baseTimestampUs()) / 1_000_000 - expectedTime) < 1e-6, `wrong B-frame at ${time}`);
      context.drawImage(frame, 0, 0, 1, 1);
      const actual = context.getImageData(0, 0, 1, 1).data;
      const expected = [[224, 48, 48], [32, 192, 96], [48, 112, 224]][Math.min(2, Math.floor(expectedTime))]!;
      assert(expected.every((value, channel) => Math.abs(value - actual[channel]!) < 10),
        `wrong decoded color at ${time}: ${Array.from(actual)} expected ${expected}; ${JSON.stringify(frame.colorSpace.toJSON())}`);
    } finally { frame.close(); }
  };
  try {
    for (let frame = 0; frame < 90; frame++) await checkFrame(frame / 30);
    assert(decodes <= 180, `sequential playback repeatedly decoded GOPs (${decodes} packets for 90 frames)`);
    report.push({ fixture: "sequential-bframes", status: "pass", frames: 90, decodedPackets: decodes }); show();
    for (const time of [2.9, 0.2, 1, 2, Infinity, -1, 0, 0]) await checkFrame(time);
    report.push({ fixture: "random-seek-origin-eof", status: "pass" }); show();
    const held = await source.frameAt(0);
    source.close();
    context.drawImage(held, 0, 0); held.close(); // Closing the source must not close its caller's frame.
    try { await source.frameAt(0); throw new Error("closed video source accepted a request"); }
    catch (error) { assert(error instanceof Error && error.message.includes("closed"), "closed source returned the wrong error"); }
    const buffered = createDecoderRing({ buffer: await blob.arrayBuffer() });
    try { const frame = await buffered.frameAt(1); assert(frame.timestamp === 4_000_000, "buffer input changed source timing"); frame.close(); }
    finally { buffered.close(); }
  } finally { VideoDecoder.prototype.decode = originalDecode; source.close(); }
  const thumbnails = await keyframeThumbnails({ blob, height: 18, maxCount: 3 });
  try {
    assert(thumbnails.length === 3 && thumbnails.every((item, i) => Math.abs(item.tS - i) < 1e-6), "keyframe thumbnails lost source rebasing");
    assert(thumbnails.every((item) => item.bitmap.width === 32 && item.bitmap.height === 18), "thumbnail dimensions changed");
  } finally { for (const item of thumbnails) item.bitmap.close(); }
  report.push({ fixture: "keyframe-thumbnails", status: "pass", count: 3 }); show();
  const audio = await decodeAudioBuffer({ blob: await (await fetch("/fixtures/source.mp4")).blob() }, new OfflineAudioContext(2, 1, 48_000));
  assert(audio.length === 144_000 && audio.numberOfChannels === 2, "AAC input did not trim priming/padding to 3 seconds");
  const values = audio.getChannelData(0);
  let best = -Infinity, lag = 0;
  for (let shift = -32; shift <= 32; shift++) {
    let dot = 0;
    for (let i = 5000; i < 9000; i++) dot += Math.sin(i * 440 * 2 * Math.PI / 48_000) * values[i + shift]!;
    if (dot > best) { best = dot; lag = shift; }
  }
  assert(Math.abs(lag) <= 1, `AAC input introduced ${lag} samples of timing offset`);
  report.push({ fixture: "aac-input-timing", status: "pass", samples: audio.length, lag }); show();
  const resampled = await decodeAudioBuffer({ blob: await (await fetch("/fixtures/tone.wav")).blob() }, new OfflineAudioContext(2, 1, 8000));
  assert(resampled.sampleRate === 8000 && resampled.length === 24_000 && resampled.numberOfChannels === 2, "audio resampling changed duration/channels");
  const rms = Math.sqrt(resampled.getChannelData(0).slice(100, 4100).reduce((sum, value) => sum + value * value, 0) / 4000);
  assert(rms > 0.2 && rms < 0.3, "resampling changed source gain");
  report.push({ fixture: "audio-resampling", status: "pass", sampleRate: 8000, samples: resampled.length, rms }); show();
}
async function checkVideoColor() {
  const config = await input("video");
  const canvas = document.createElement("canvas"); canvas.width = 640; canvas.height = 360;
  const context = canvas.getContext("2d")!;
  const source = createDecoderRing({ blob: await (await fetch("/fixtures/source.mp4")).blob() });
  const pixels: Array<{ path: string; time: number; drawn: number[]; copied?: number[]; colorSpace?: VideoColorSpaceInit }> = [];
  try {
    for (const time of [0.5, 1.5, 2.5]) {
      const frame = await source.frameAt(time);
      try {
        context.drawImage(frame, 0, 0);
        const drawn = Array.from(context.getImageData(320, 180, 1, 1).data);
        const rgba = new Uint8Array(640 * 360 * 4);
        await frame.copyTo(rgba, { format: "RGBA", colorSpace: "srgb" });
        pixels.push({ path: "decoded", time, drawn, copied: Array.from(rgba.subarray((180 * 640 + 320) * 4, (180 * 640 + 320) * 4 + 4)), colorSpace: frame.colorSpace.toJSON() });
      } finally { frame.close(); }
    }
  } finally { source.close(); }
  for (const gpu of [false, true]) {
    const player = new BrowserValleWebPlayer({ ...config, canvas, gpu, audioContext: null });
    try {
      await player.init();
      for (const frame of [15, 45, 75]) {
        await player.renderFrame(frame);
        pixels.push({ path: gpu ? "gpu" : "cpu", time: frame / 30, drawn: Array.from(context.getImageData(320, 180, 1, 1).data) });
      }
    } finally { await player.close(); }
  }
  for (const pixel of pixels) {
    const expected = [[224, 48, 48], [32, 192, 96], [48, 112, 224]][Math.floor(pixel.time)]!;
    for (const actual of [pixel.drawn, pixel.copied]) {
      if (!actual) continue;
      assert(expected.every((value, channel) => Math.abs(value - actual[channel]!) < 10),
        `${pixel.path} changed video color at ${pixel.time}: ${actual}`);
    }
  }
  report.push({ fixture: "video-color", status: "pass", pixels }); show();
}

async function checkDirectAudioPcm() {
  const config = await input("av");
  const player = new BrowserValleWebPlayer({ ...config, canvas: document.createElement("canvas"), audioContext: null });
  try {
    await player.init();
    const engine = (player as any).engine;
    const id = player.renderInfo.renderId;
    const sources = JSON.parse(engine.audio_sources_json(id, 0n, 8192n));
    assert(sources.length === 1, "audio requirements expanded beyond the active source");
    const digest = sources[0].digest;
    const decoded = await decodeAudioBuffer({ blob: await (await fetch("/fixtures/tone.wav")).blob() }, new OfflineAudioContext(2, 1, 48_000));
    const left = decoded.getChannelData(0), right = decoded.getChannelData(1);
    const register = () => engine.register_audio_pcm(id, digest, left, right);
    assert(!engine.has_audio_pcm(id, digest), "new render inherited decoded PCM");
    register();
    assert(engine.has_audio_pcm(id, digest), "registered PCM was not retained");
    let jsonChars = 0;
    for (let start = 0; start < decoded.length; start += 8192) {
      const end = Math.min(decoded.length, start + 8192);
      jsonChars += engine.audio_sources_json(id, BigInt(start), BigInt(end)).length;
      const mixed = engine.mix_audio_pcm(id, BigInt(start), BigInt(end));
      assert(mixed instanceof Float32Array && mixed.length === (end - start) * 2, "WASM returned the wrong PCM shape");
      for (let index = start; index < end; index++) {
        assert(mixed[(index - start) * 2] === Math.fround(left[index]! * 0.5)
          && mixed[(index - start) * 2 + 1] === Math.fround(right[index]! * 0.5), `PCM gain/sample drift at ${index}`);
      }
    }
    const seek = engine.mix_audio_pcm(id, 47000n, 53000n);
    for (let index = 0; index < 6000; index++) assert(seek[index * 2] === Math.fround(left[index + 47000]! * 0.5), "random seek changed PCM");
    assert(jsonChars < 10_000 && typeof engine.audio_block_json === "undefined", "per-sample JSON is still in the runtime");
    report.push({ fixture: "wasm-pcm-bit-exact", status: "pass", samples: decoded.length, jsonChars }); show();
    const rejects = (run: () => unknown) => { let failed = false; try { run(); } catch { failed = true; } assert(failed, "invalid audio input was accepted"); };
    rejects(() => engine.audio_sources_json(id, -1n, 1n));
    rejects(() => engine.mix_audio_pcm(id, 0n, 8193n));
    rejects(() => engine.mix_audio_pcm(id, 144000n, 144001n));
    rejects(() => engine.register_audio_pcm(id, digest, left, new Float32Array(0)));
    rejects(() => engine.register_audio_pcm(id, `sha256:${"0".repeat(64)}`, left, right));
    engine.register_audio_pcm(id, digest, new Float32Array([NaN]), new Float32Array([0]));
    rejects(() => engine.mix_audio_pcm(id, 0n, 1n));
    register();
    engine.open_fixed_package(config.fixedPackageManifestJson, config.timelineJson, config.resourceManifestJson, config.verifiedBindingBundleJson);
    assert(!engine.has_audio_pcm(id, digest), "package replacement retained old decoded PCM");
    rejects(() => engine.mix_audio_pcm(id, 0n, 1n));
    register();
    assert(engine.mix_audio_pcm(id, 0n, 1n).length === 2, "audio did not recover after fresh admission");
    report.push({ fixture: "audio-scope-and-validation", status: "pass" }); show();
  } finally { player.close(); }
}

async function checkDeliverySettings() {
  const videoConfigurations: VideoEncoderConfig[] = [];
  const audioConfigurations: AudioEncoderConfig[] = [];
  const configureVideo = VideoEncoder.prototype.configure;
  const configureAudio = AudioEncoder.prototype.configure;
  VideoEncoder.prototype.configure = function(config) {
    videoConfigurations.push({ ...config });
    return configureVideo.call(this, config);
  };
  AudioEncoder.prototype.configure = function(config) {
    audioConfigurations.push({ ...config });
    return configureAudio.call(this, config);
  };
  try {
    const resized = await exportBrowserVideo(await input("av"), {
      width: 320, height: 240, frameRate: 24, videoBitrate: 700_000, audioBitrate: 128_000,
    });
    assert(resized.width === 320 && resized.height === 240 && resized.frameCount === 72 && resized.hasAudio, "resized delivery ignored settings");
    const videoConfig = videoConfigurations.at(-1)!;
    const audioConfig = audioConfigurations.at(-1)!;
    assert(videoConfig.width === 320 && videoConfig.height === 240 && videoConfig.framerate === 24
      && videoConfig.bitrate === 700_000, "selected video parameters did not reach the browser encoder");
    assert(audioConfig.bitrate === 128_000, "selected audio bitrate did not reach the encoder");
    await save("settings-downscale", resized);
    report.push({ fixture: "settings-downscale", status: "pass", width: resized.width, height: resized.height,
      frames: resized.frameCount, frameRate: resized.frameRate, videoConfig, audioConfig }); show();
    audioConfigurations.length = 0;
    const silent = await exportBrowserVideo(await input("av"), {
      width: 1280, height: 720, frameRate: 60, videoBitrate: 2_000_000, includeAudio: false,
    });
    assert(silent.width === 1280 && silent.height === 720 && silent.frameCount === 180 && !silent.hasAudio, "upscaled silent delivery ignored settings");
    assert(audioConfigurations.length === 0, "muted delivery still encoded audio");
    await save("settings-upscale-silent", silent);
    report.push({ fixture: "settings-upscale-silent", status: "pass", width: silent.width, height: silent.height,
      frames: silent.frameCount, frameRate: silent.frameRate, videoConfig: videoConfigurations.at(-1), audioConfigurations: audioConfigurations.length }); show();
    const fractional = await exportBrowserVideo(await input("fractional"), { frameRate: 25, videoBitrate: 500_000 });
    assert(fractional.frameCount === 26 && fractional.durationS === 1.001, "FPS conversion changed fractional work duration");
    await save("settings-fractional-fps", fractional);
    report.push({ fixture: "settings-fractional-fps", status: "pass", frames: fractional.frameCount,
      frameRate: fractional.frameRate, durationS: fractional.durationS }); show();
  } finally {
    VideoEncoder.prototype.configure = configureVideo;
    AudioEncoder.prototype.configure = configureAudio;
  }
}
try {
  for (const fixture of ["motion", "av", "short-av", "fractional", "video", "video-av"]) {
    const config = await input(fixture);
    const frames: number[] = [];
    const result = await exportBrowserVideo(config, { onProgress(value) {
      if (value.phase === "rendering") frames.push(value.framesCompleted);
    } });
    assert(result.frameCount === (fixture === "fractional" ? 30 : fixture === "short-av" ? 1 : 90), `${fixture}: unexpected frame count ${result.frameCount}`);
    assert(frames.length === result.frameCount && frames.every((frame, i) => frame === i + 1), "export skipped or duplicated a frame");
    assert(result.width === 640 && result.height === 360, "export dimensions changed");
    assert(result.hasAudio === (fixture === "av" || fixture === "short-av" || fixture === "video-av"), "audio track presence changed");
    await save(fixture, result);
    report.push({ fixture, status: "pass", ...result, blob: { size: result.blob!.size } }); show();
  }
  await checkMediaReaders();
  await checkVideoColor();
  await checkDirectAudioPcm();
  await checkDeliverySettings();
  const controller = new AbortController();
  let cancelledFrames = 0;
  try {
    await exportBrowserVideo(await input("long"), { signal: controller.signal, onProgress(value) {
      if (value.framesCompleted === 12) { cancelledFrames = value.framesCompleted; controller.abort(); }
    } });
    throw new Error("cancelled export unexpectedly succeeded");
  } catch (error) {
    assert(controller.signal.aborted && error instanceof DOMException && error.name === "AbortError", "cancellation did not propagate AbortError");
  }
  report.push({ fixture: "cancel", status: "pass", cancelledFrames }); show();
  const retried = await exportBrowserVideo(await input("fractional"));
  await save("retry", retried);
  report.push({ fixture: "retry-after-cancel", status: "pass", frameCount: retried.frameCount }); show();
  const chunks: Array<{ position: number; data: Uint8Array }> = [];
  let committed = false;
  const writable = new WritableStream({ write(chunk: { position: number; data: Uint8Array }) {
    chunks.push({ position: chunk.position, data: chunk.data.slice() });
  }, close() { committed = true; } }) as FileSystemWritableFileStream;
  const streamed = await exportBrowserVideo(await input("av"), { writable });
  assert(streamed.blob === null && committed && !writable.locked, "file export did not commit and release its writer");
  const bytes = new Uint8Array(Math.max(...chunks.map((chunk) => chunk.position + chunk.data.length)));
  for (const chunk of chunks) bytes.set(chunk.data, chunk.position);
  await save("streamed", { ...streamed, blob: new Blob([bytes], { type: "video/mp4" }) });
  report.push({ fixture: "streamed-file", status: "pass", bytes: bytes.length }); show();
  const cancelled = new AbortController();
  let fileCommitted = false, fileAborted = false;
  const cancelledFile = new WritableStream({ close() { fileCommitted = true; }, abort() { fileAborted = true; } }) as FileSystemWritableFileStream;
  try {
    await exportBrowserVideo(await input("long"), { writable: cancelledFile, signal: cancelled.signal, onProgress(value) {
      if (value.framesCompleted === 12) cancelled.abort();
    } });
    throw new Error("cancelled file export unexpectedly succeeded");
  } catch (error) {
    assert(error instanceof DOMException && error.name === "AbortError", "file cancellation did not propagate AbortError");
  }
  assert(fileAborted && !fileCommitted && !cancelledFile.locked, "cancelled file was committed or retained its writer");
  report.push({ fixture: "cancelled-file", status: "pass" }); show();
  const finishing = new AbortController();
  let finalCommitted = false, finalAborted = false;
  const finalFile = new WritableStream({ close() { finalCommitted = true; }, abort() { finalAborted = true; } }) as FileSystemWritableFileStream;
  try {
    await exportBrowserVideo(await input("fractional"), { writable: finalFile, signal: finishing.signal, onProgress(value) {
      if (value.phase === "finalizing") finishing.abort();
    } });
    throw new Error("cancellation during finalization unexpectedly succeeded");
  } catch (error) {
    assert(error instanceof DOMException && error.name === "AbortError", "finalization cancellation did not propagate AbortError");
  }
  assert(finalAborted && !finalCommitted && !finalFile.locked, "cancelled finalization committed a file");
  report.push({ fixture: "cancelled-finalization", status: "pass" }); show();
  try {
    await exportBrowserVideo(await input("odd"));
    throw new Error("odd dimensions unexpectedly succeeded");
  } catch (error) {
    assert(error instanceof Error && error.message.includes("even canvas"), "odd dimensions did not produce the expected capability error");
  }
  report.push({ fixture: "odd-dimensions", status: "pass" }); show();
  document.title = "PASS · Browser export checks";
} catch (error) {
  report.push({ status: "fail", message: error instanceof Error ? error.stack : String(error) }); show();
  document.title = "FAIL · Browser export checks";
} finally {
  await fetch("/__test/artifacts/report.json", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(report, null, 2) });
}
