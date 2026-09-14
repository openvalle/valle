import type { MotionContext } from "./host.ts";

export type GoodMotionContext = Extract<MotionContext, { status: "ok" }>;

/**
 * Adapt the producer-owned fixed package to the shared Player input.
 *
 * Draft controls are sent to the producer, which emits a new immutable fixed package. This
 * adapter must not rebuild or quantize Timeline data in JavaScript, because doing so would detach
 * the document from its fixed manifest, ResourceManifest and verified binding bundle pins.
 */
export function buildMotionPreview(context: GoodMotionContext) {
  return {
    fixedPackageManifestJson: context.fixedPackageManifestJson,
    timelineJson: context.timelineJson,
    resourceManifestJson: context.resourceManifestJson,
    verifiedBindingBundleJson: context.verifiedBindingBundleJson,
    assets: context.resourceLocators,
    runtimeAssets: context.runtimeAssets,
    runtimeBaseUrl: context.runtimeBaseUrl,
  };
}
