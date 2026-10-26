import {
  resourceByteRange,
  type MotionCompileOptions,
  type PreviewResourceInput,
} from "valle-engine";
import type { MotionContext } from "./host.ts";
import { frozenShaderBytes } from "./shader-input.ts";

export interface StandaloneAssets {
  bindings: Array<{ name: string; path: string; alias: string }>;
  resourceInputs: PreviewResourceInput[];
  locators: Array<{ id: string; url: string }>;
  options: MotionCompileOptions;
  resourceBytes?: Uint8Array;
}

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("Motion asset package is incomplete");
  }
  return value as Record<string, unknown>;
}

/** Reuse the Host's frozen media facts; the Worker owns new JSX and Timeline compilation. */
export function standaloneAssetsFromMotionContext(
  specs: readonly string[],
  context: MotionContext,
): StandaloneAssets {
  if (context.status !== "ok") {
    throw new Error(context.diagnostics.map((item) => item.message).join("\n"));
  }
  const entries = record(context.resourceManifest.entries);
  const bundle = record(JSON.parse(context.verifiedBindingBundleJson));
  const bindings = record(bundle.bindings);
  const names = new Set<string>();
  const assetBindings: StandaloneAssets["bindings"] = [];
  const resourceInputs: PreviewResourceInput[] = [];
  const resources: NonNullable<MotionCompileOptions["resources"]>[number][] = [];
  const fontAliases: Record<string, Uint8Array> = {};
  const shaders: NonNullable<MotionCompileOptions["shaders"]>[number][] = [];
  for (const spec of specs) {
    const equals = spec.indexOf("=");
    if (equals <= 0 || equals === spec.length - 1)
      throw new Error(`Invalid Motion asset binding: ${spec}`);
    const name = spec.slice(0, equals);
    const path = spec.slice(equals + 1);
    if (names.has(name)) throw new Error(`Duplicate Motion asset binding: ${name}`);
    names.add(name);
    const nativeId = `asset:${name}`;
    const alias = `asset_${assetBindings.length}`;
    const entry = record(entries[nativeId]);
    const binding = record(bindings[nativeId]);
    const facts = record(binding.facts);
    if (typeof entry.digest !== "string")
      throw new Error(`Motion asset ${name} has no content digest`);
    const digest = entry.digest;
    if (Array.isArray(binding.dependencies) && binding.dependencies.length) {
      throw new Error(
        `Motion asset ${name} has dependent resource facts not available in browser preparation`,
      );
    }
    if (entry.kind === "shader") {
      const bytes = context.resourceBytes ?? new Uint8Array();
      shaders.push({
        assetControl: name,
        frozenBytes: frozenShaderBytes(
          resourceByteRange(facts.manifestBytes, bytes),
          resourceByteRange(facts.sourceBytes, bytes),
        ),
      });
    }
    assetBindings.push({ name, path, alias });
    resourceInputs.push({ id: `resource:${alias}`, entry, facts });
    resources.push({ control: name, contentHash: digest });
    if (entry.kind === "font") {
      fontAliases[`asset://${name}`] = resourceByteRange(
        facts.bytes,
        context.resourceBytes ?? new Uint8Array(),
      );
    }
  }
  const locators = context.resourceLocators.flatMap(({ id, url }) => {
    const found = assetBindings.find((asset) => id === `asset:${asset.name}`);
    return found ? [{ id: `resource:${found.alias}`, url }] : [];
  });
  return {
    bindings: assetBindings,
    resourceBytes: context.resourceBytes,
    resourceInputs,
    locators,
    options: {
      resources,
      ...(Object.keys(fontAliases).length ? { fontAliases } : {}),
      ...(shaders.length ? { shaders } : {}),
    },
  };
}
