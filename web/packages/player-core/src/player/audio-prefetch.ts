/**
 * Timeline audio clips may consume a video container's audio stream. Compile already
 * admits that pairing; WebAudio still has to fetch the same asset id.
 */
export function assetSuppliesAudio(type: string): boolean {
  return type === "audio" || type === "video";
}
