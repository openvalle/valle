export {
  createBrowserValleWebPlayer,
  BrowserValleWebPlayer,
  type BrowserResourceLocator,
  type BrowserResourceCacheLimits,
  type BrowserValleWebPlayerOptions,
  type CanonicalTimelineDocument,
  type TimelineDocumentView,
  type TimelineSequenceView,
} from "./player/controller.ts";
export { createDecoderRing, keyframeThumbnails } from "./media/decoder-ring.ts";
export { resolvePlayerRuntimeAssets, type PlayerRuntimeAssets } from "./runtime-assets.ts";
export {
  canonicalizeTimelineDocumentWithWasm,
  compileTimelineWithWasm,
  createTimelineCompilerRuntime,
  type TimelineCompilerRuntime,
  type TimelineCompilerRuntimeOptions,
} from "./timeline-compiler.ts";

export const VALLE_PLAYER_CORE_SKELETON = "valle-player-core@0.0.0";
