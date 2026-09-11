import { describe, expect, test } from "bun:test";
import type { Timeline } from "@valle/engine";
import type { TimelineDocument } from "@valle/engine/internal";

import {
  MotionFileHost,
  ProjectHost,
  TimelineFileHost,
  assertMotionContext,
  assertStudioBoot,
  loadStudioHost,
  projectMotionContextFromAdmittedPreview,
} from "./host.ts";
import {
  initializeTimelineWorkspaceRuntime,
  type TimelineWorkspaceRuntimeConfig,
} from "./timeline-workspace-runtime.ts";

const runtime = {
  assetBaseUrl: "/assets/",
  proxyBase: "/proxy/",
  assetUrls: {
    engineGlue: "/runtime/engine/valle_engine.js",
    engineWasm: "/runtime/engine/valle_engine_bg.wasm",
    canvasKitFullGlue: "/runtime/canvaskit/canvaskit.js",
    canvasKitFullWasm: "/runtime/canvaskit/canvaskit.wasm",
    defaultSansFont: "/runtime/fonts/NotoSans-Regular.ttf",
    productFrameWorker: "/runtime/workers/product-frame.js",
  },
};
const capabilities = {
  saveTimeline: false,
  editProject: false,
  editMotionProps: false,
  writeMotionSource: false,
};

function json(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function canonicalTimeline(motion = false): TimelineDocument {
  const source = motion
    ? {
        type: "motion",
        component: "component:Card",
        props: {},
        cues: {},
        resources: { model: "asset:product" },
        phases: { enterDuration: null, exitDuration: null },
        sourceStart: "0/1",
        sourceDuration: "2/1",
        rate: "1/1",
        endBehavior: "hold",
      }
    : { type: "solid", color: "#000000" };
  return {
    document: {
      canvas: {
        width: 640,
        height: 360,
        fps: "30/1",
        duration: "2/1",
        sampleRate: 48_000,
        channelLayout: "stereo",
        colorSpace: "srgb",
      },
      background: { color: "#000000" },
      visual: {
        tracks: [{ id: "visual:main", items: [{
          type: "clip",
          id: "hero",
          duration: "2/1",
          source,
          layer: {
            transform: {
              position: { type: "constant", value: [0, 0] },
              scale: { type: "constant", value: [1, 1] },
              rotation: { type: "constant", value: 0 },
              anchor: [0.5, 0.5],
            },
            opacity: { type: "constant", value: 1 },
            blend: "normal",
            mask: null,
            filters: [],
          },
        }] }],
      },
      audio: { tracks: [] },
      captions: { tracks: [] },
      adjustments: [],
      camera: null,
      metadata: {},
    },
  } as TimelineDocument;
}

function sparseTimeline(): Timeline {
  return {
    canvas: { width: 640, height: 360, fps: 30, background: "#000000ff" },
    tracks: {
      visual: [{
        clips: [{ start: 0, duration: 2, kind: "solid", color: "#000000ff" }],
      }],
    },
  } as unknown as Timeline;
}

function fixedPackage() {
  const timeline = sparseTimeline();
  const renderTimeline = canonicalTimeline();
  const resourceManifest = {
    entries: {},
  };
  return {
    timelineRevision: {
      revision: 1,
      parentRevision: null,
      createdAt: "2026-01-01T00:00:00.000Z",
      actor: "test",
      cause: { type: "genesis" },
      intent: null,
    },
    timeline,
    timelineJson: JSON.stringify(timeline),
    render: {
      timeline: renderTimeline,
      timelineJson: JSON.stringify(renderTimeline),
      fixedPackageManifestJson: "{\"format\":\"valle.fixed-render-package@1\"}",
      resourceManifestJson: JSON.stringify(resourceManifest),
      resourceManifest,
      verifiedBindingBundleJson: JSON.stringify({ bindings: {}, capabilities: {} }),
    },
  };
}

describe("Studio host adapters", () => {
  test("selects all three hosts only from the explicit session discriminator", async () => {
    const fixtures = [
      [{ kind: "project", projectId: "p1", revision: 3, token: "tok" }, ProjectHost],
      [{ kind: "timeline-file", input: "cut.valle.json" }, TimelineFileHost],
      [{ kind: "motion-file", input: "card.motion.tsx", generation: 2 }, MotionFileHost],
    ] as const;
    for (const [session, Host] of fixtures) {
      const fetcher = async () => json({ protocolVersion: 1, session, capabilities, runtime });
      const host = await loadStudioHost("", fetcher);
      expect(host).toBeInstanceOf(Host);
      expect(host.boot.session.kind).toBe(session.kind);
    }
  });

  test("rejects malformed Studio event JSON", () => {
    const boot = assertStudioBoot({
      protocolVersion: 1,
      session: { kind: "timeline-file", input: "cut.valle.json" },
      capabilities,
      runtime,
    });
    const listeners = new Map<string, (event: MessageEvent<string>) => void>();
    let closed = false;
    const host = new TimelineFileHost(
      boot,
      async () => json({}),
      () => ({
        addEventListener: (type, listener) => listeners.set(type, listener),
        close: () => {
          closed = true;
        },
      }),
    );
    const received: unknown[] = [];
    const unsubscribe = host.subscribe((event) => received.push(event));
    const timelineListener = listeners.get("timeline");
    if (!timelineListener) throw new Error("timeline event listener was not registered");

    expect(() => timelineListener({ data: "not-json" } as MessageEvent<string>)).toThrow(
      "Studio timeline event payload must be valid JSON",
    );
    expect(received).toEqual([]);
    expect(closed).toBe(true);
    unsubscribe();
  });

  test("project host submits the generated full-document edit contract", async () => {
    const calls: Array<{ url: string; init?: RequestInit }> = [];
    const boot = assertStudioBoot({
      protocolVersion: 1,
      session: { kind: "project", projectId: "p 1", revision: 7, token: "secret" },
      capabilities: { ...capabilities, saveTimeline: true, editProject: true },
      runtime,
    });
    const fetcher = (async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input);
      calls.push({ url, init });
      return url.startsWith("/timeline/get")
        ? json(fixedPackage())
        : json({
            outcome: "committed",
            revision: 8,
          });
    });
    const host = new ProjectHost(
      boot as typeof boot & { session: Extract<typeof boot.session, { kind: "project" }> },
      fetcher,
    );

    await host.loadTimeline();
    const edit = {
      baseRevision: 7,
      timeline: sparseTimeline(),
      intent: "studio save",
    };
    const report = await host.saveTimeline(edit);

    expect(calls[0]?.url).toBe("/timeline/get?project=p%201");
    expect(calls[1]?.url).toBe("/timeline/edit?project=p%201");
    expect(new Headers(calls[1]?.init?.headers).get("x-valle-token")).toBe("secret");
    expect(JSON.parse(String(calls[1]?.init?.body))).toEqual(edit);
    expect(report).toMatchObject({ outcome: "committed", revision: 8 });
  });

  test("Timeline host fails closed on the unsupported versioned track-list shape", async () => {
    const boot = assertStudioBoot({
      protocolVersion: 1,
      session: { kind: "timeline-file", input: "timeline.json" },
      capabilities,
      runtime,
    });
    const current = fixedPackage();
    const unsupportedTimeline = {
      version: 2,
      canvas: { width: 640, height: 360, fps: 30 },
      tracks: [{ type: "visual", clips: [] }],
    };
    const host = new TimelineFileHost(boot, async () => json({
      ...current,
      timeline: unsupportedTimeline,
      timelineJson: JSON.stringify(unsupportedTimeline),
      runtimeAssets: {},
      assetBaseUrl: "/assets/",
    }));

    await expect(host.loadTimeline()).rejects.toThrow("complete authoring and render projections");
  });

  test("Timeline host rejects partial fixed render packages", async () => {
    const boot = assertStudioBoot({
      protocolVersion: 1,
      session: { kind: "timeline-file", input: "timeline.json" },
      capabilities,
      runtime,
    });
    const current = fixedPackage();
    const {
      fixedPackageManifestJson,
      resourceManifestJson,
      resourceManifest,
      verifiedBindingBundleJson,
      ...projection
    } = current.render;
    const base = {
      ...current,
      render: projection,
      preview: { status: "unavailable", code: "verified_binding_bundle_unavailable" },
      runtimeAssets: {},
      assetBaseUrl: "/assets/",
    };
    for (const render of [
      { ...projection, resourceManifest },
      { ...projection, fixedPackageManifestJson },
      { ...projection, fixedPackageManifestJson, resourceManifestJson, resourceManifest },
      { ...projection, verifiedBindingBundleJson },
    ]) {
      const host = new TimelineFileHost(boot, async () => json({ ...base, render }));
      await expect(host.loadTimeline()).rejects.toThrow(
        "every fixed-package member and manifest together",
      );
    }
    const extraProjection = new TimelineFileHost(boot, async () => json({
      ...base,
      render: { ...current.render, extraIdentity: "producer-private" },
    }));
    await expect(extraProjection.loadTimeline()).rejects.toThrow(
      "complete authoring and render projections",
    );
  });

  test("project host loads and saves an authoritative snapshot with preview explicitly unavailable", async () => {
    const boot = assertStudioBoot({
      protocolVersion: 1,
      session: { kind: "project", projectId: "p1", revision: 7, token: "secret" },
      capabilities: { ...capabilities, saveTimeline: true, editProject: true },
      runtime,
    });
    const current = fixedPackage();
    const {
      fixedPackageManifestJson: _fixedPackageManifest,
      resourceManifestJson: _resourceManifestJson,
      resourceManifest: _resourceManifest,
      verifiedBindingBundleJson: _bundle,
      ...render
    } = current.render;
    const snapshot = { ...current, render };
    const calls: string[] = [];
    const host = new ProjectHost(
      boot as typeof boot & { session: Extract<typeof boot.session, { kind: "project" }> },
      async (input) => {
        const url = String(input);
        calls.push(url);
        return url.startsWith("/timeline/get")
          ? json({
              ...snapshot,
              preview: { status: "unavailable", code: "verified_binding_bundle_unavailable" },
            })
          : json({
              outcome: "unchanged",
              revision: 1,
            });
      },
    );

    const loaded = await host.loadTimeline();
    expect(loaded.preview).toEqual({
      status: "unavailable",
      code: "verified_binding_bundle_unavailable",
    });
    expect(loaded.render.verifiedBindingBundleJson).toBeUndefined();
    expect(loaded.render.resourceManifest).toBeUndefined();
    let previewLoads = 0;
    const workspace = await initializeTimelineWorkspaceRuntime(
      loaded as TimelineWorkspaceRuntimeConfig,
      {
        load: async () => {
          previewLoads += 1;
        },
      },
      async ({ runtimeAssets }) => {
        expect(runtimeAssets).toEqual({
          engine: {
            glue: runtime.assetUrls.engineGlue,
            wasm: runtime.assetUrls.engineWasm,
          },
          canvasKit: {
            full: {
              glue: runtime.assetUrls.canvasKitFullGlue,
              wasm: runtime.assetUrls.canvasKitFullWasm,
            },
          },
          fonts: { defaultSans: runtime.assetUrls.defaultSansFont },
          workers: { productFrame: runtime.assetUrls.productFrameWorker },
        });
        return {
          normalizeTimeline: (timeline) => timeline,
          timelineTimeFromFrames: (frames, fps) => frames / Number(fps),
          timelineSourceTimeDeltaFromFrames: (frames, fps, rate = 1) => (
            frames / Number(fps) * (rate ?? 1)
          ),
          compileTimeline: () => ({
            timeline: loaded.render.timeline,
            timelineJson: loaded.render.timelineJson,
            view: {
              canvas: { width: 640, height: 360, durationSeconds: 2, framesPerSecond: 30, frameCount: 60, sampleRate: 48_000, sampleCount: 96_000 },
              sequences: [],
            },
          }),
          canonicalizeTimelineDocument: () => ({
            timeline: loaded.render.timeline,
            timelineJson: loaded.render.timelineJson,
            view: {
              canvas: { width: 640, height: 360, durationSeconds: 2, framesPerSecond: 30, frameCount: 60, sampleRate: 48_000, sampleCount: 96_000 },
              sequences: [],
            },
          }),
        };
      },
    );
    expect(workspace.previewAvailable).toBe(false);
    expect(previewLoads).toBe(0);
    const saved = await host.saveTimeline({
      baseRevision: loaded.timelineRevision.revision,
      timeline: loaded.timeline,
      intent: "authoring without preview",
    });
    expect(saved).toMatchObject({ outcome: "unchanged", revision: 1 });
    expect(calls).toEqual([
      "/timeline/get?project=p1",
      "/timeline/edit?project=p1",
    ]);
  });

  test("project host derives a selected clip MotionContext from admitted structure and Timeline", async () => {
    const boot = assertStudioBoot({
      protocolVersion: 1,
      session: { kind: "project", projectId: "p1", revision: 7, token: "secret" },
      capabilities: { ...capabilities, editMotionProps: true },
      runtime: { ...runtime, assetBaseUrl: "/passets/p1/" },
    });
    const timeline = canonicalTimeline(true);
    const hero = timeline.document.visual.tracks[0]?.items[0];
    if (!hero || hero.type !== "clip" || hero.source.type !== "motion") {
      throw new Error("expected Motion fixture");
    }
    hero.source.resources = { poster: "asset:poster" };
    hero.source.cues = {
      beat: {
        type: "source-range",
        start: "1/3",
        end: "4/3",
        enterDuration: "1/10",
        exitDuration: "1/10",
      },
      title: {
        type: "source-range",
        start: "1/2",
        end: "7/6",
        enterDuration: "1/10",
        exitDuration: "1/10",
      },
    };
    const artifactDigest = `sha256:${"a".repeat(64)}`;
    const fontDigest = `sha256:${"b".repeat(64)}`;
    const verifiedArtifact = {
      formatVersion: 1,
      component: "Card",
      controls: {
        props: {},
        data: {},
        assets: { poster: { kind: "image", required: true } },
        timing: {
          enterFrames: { default: 1 },
          holdCycleFrames: { default: null },
          exitFrames: { default: 1 },
        },
        cues: {},
        camera: { values: {} },
      },
      capabilitySet: {
        names: [
          "backdrop-filter",
          "blend",
          "box",
          "clip-mask",
          "cue-signals",
          "dynamic-path",
          "filter",
          "geometry-path",
          "glow",
          "gradient-paint",
          "group",
          "image",
          "path",
          "svg",
          "tailwind",
          "text",
          "text-on-path",
          "text-per-unit",
          "text-split",
        ],
      },
    };
    const config = {
      ...fixedPackage(),
      generation: 9,
      render: {
        ...fixedPackage().render,
        timelineJson: JSON.stringify(timeline),
        timeline,
        resourceManifest: {
          entries: {
            "component:Card": { kind: "motion-artifact", digest: artifactDigest },
            "font:noto": { kind: "font", digest: fontDigest },
          },
        },
        verifiedBindingBundleJson: JSON.stringify({
          bindings: {
            "component:Card": {
              digest: artifactDigest,
              handle: 1,
              facts: { kind: "motion-artifact", artifact: verifiedArtifact },
              dependencies: [],
            },
          },
          capabilities: {},
        }),
      },
      assets: [
        { id: "poster", type: "image", url: "images/poster.png" },
        { id: "font:noto", type: "font", url: "fonts/noto.otf" },
      ],
      runtimeAssets: { engine: {} },
      motion: {
        structures: [{
          clipId: "hero",
          // The structure is an authoring projection, not an artifact trust source.
          artifact: {},
          authoring: {
            props: {},
            timing: { enterFrames: 4, exitFrames: 5 },
            cues: {
              beat: { startFrame: 10, endFrame: 40, enterFrames: 3, exitFrames: 3 },
              title: { startFrame: 15, endFrame: 35, enterFrames: 2, exitFrames: 4 },
            },
            sourceMap: { version: 1, component: "Card", entry: "components/Card.tsx", closureDigest: `sha256:${"c".repeat(64)}`, modules: [], nodes: [], exprs: [], objects: [] },
            totalFrames: 60,
          },
        }],
        shaders: [],
        problems: [],
      },
    };
    const host = new ProjectHost(
      boot as typeof boot & { session: Extract<typeof boot.session, { kind: "project" }> },
      async () => json(config),
    );

    const context = projectMotionContextFromAdmittedPreview(
      await host.loadTimeline(),
      host.boot,
      { clipId: "hero" },
    );

    expect(context.status).toBe("ok");
    if (context.status !== "ok") throw new Error("expected project MotionContext");
    expect(context).toMatchObject({
      generation: 9,
      input: "components/Card.tsx",
      artifactDigest,
      timing: { enterFrames: 4, exitFrames: 5 },
      assets: [{ name: "poster", kind: "image", url: "/passets/p1/images/poster.png" }],
      cueBindings: {
        beat: { type: "sourceRange", startFrame: 10, endFrame: 40, enterFrames: 3, exitFrames: 3 },
        title: { type: "sourceRange", startFrame: 15, endFrame: 35, enterFrames: 2, exitFrames: 4 },
      },
      preparedData: {},
      dataSource: null,
    });
    expect(context.artifact).toEqual(verifiedArtifact);

    const withoutVerifiedArtifact = structuredClone(config);
    const artifactEntries = withoutVerifiedArtifact.render.resourceManifest.entries as Record<string, unknown>;
    delete artifactEntries["component:Card"];
    const artifactRejectedHost = new ProjectHost(
      boot as typeof boot & { session: Extract<typeof boot.session, { kind: "project" }> },
      async () => json(withoutVerifiedArtifact),
    );
    const artifactRejected = projectMotionContextFromAdmittedPreview(
      await artifactRejectedHost.loadTimeline(),
      artifactRejectedHost.boot,
      { clipId: "hero" },
    );
    expect(artifactRejected).toMatchObject({
      status: "error",
      diagnostics: [{ message: "Selected project clip is missing verified Motion artifact digest for 'component:Card'" }],
    });

    const mismatchedBinding = structuredClone(config);
    const bundle = JSON.parse(mismatchedBinding.render.verifiedBindingBundleJson) as {
      bindings: Record<string, { digest: string }>;
    };
    bundle.bindings["component:Card"].digest = `sha256:${"c".repeat(64)}`;
    mismatchedBinding.render.verifiedBindingBundleJson = JSON.stringify(bundle);
    const bindingRejectedHost = new ProjectHost(
      boot as typeof boot & { session: Extract<typeof boot.session, { kind: "project" }> },
      async () => json(mismatchedBinding),
    );
    const bindingRejected = projectMotionContextFromAdmittedPreview(
      await bindingRejectedHost.loadTimeline(),
      bindingRejectedHost.boot,
      { clipId: "hero" },
    );
    expect(bindingRejected).toMatchObject({
      status: "error",
      diagnostics: [{ message: "Selected project clip is missing Motion artifact for 'component:Card' bound by the verified package" }],
    });
  });

  test("project host fails closed when the selected clip lacks source-map format 1", async () => {
    const boot = assertStudioBoot({
      protocolVersion: 1,
      session: { kind: "project", projectId: "p1", revision: 7, token: "secret" },
      capabilities,
      runtime,
    });
    const host = new ProjectHost(
      boot as typeof boot & { session: Extract<typeof boot.session, { kind: "project" }> },
      async () => json({
        ...fixedPackage(),
        render: {
          ...fixedPackage().render,
          timelineJson: JSON.stringify(canonicalTimeline(true)),
          timeline: canonicalTimeline(true),
        },
        motion: { structures: [{ clipId: "hero", artifact: {}, authoring: {} }] },
      }),
    );

    const context = projectMotionContextFromAdmittedPreview(
      await host.loadTimeline(),
      host.boot,
      { clipId: "hero" },
    );
    expect(context).toMatchObject({ status: "error", input: "hero" });
  });

  test("fails closed on a missing or unknown session kind", () => {
    expect(() => assertStudioBoot({ protocolVersion: 1 })).toThrow("contain session");
    expect(() => assertStudioBoot({
      protocolVersion: 1,
      session: { kind: "guessed-from-fields" },
      capabilities,
      runtime,
    })).toThrow("unknown Studio session kind");
  });

  test("project boot accepts only positive JavaScript-safe numeric revisions", () => {
    const projectBoot = (revision: unknown) => ({
      protocolVersion: 1,
      session: { kind: "project", projectId: "p1", revision, token: "secret" },
      capabilities,
      runtime,
    });

    expect(assertStudioBoot(projectBoot(1)).session).toMatchObject({ revision: 1 });
    for (const revision of ["sha256:r1", 0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
      expect(() => assertStudioBoot(projectBoot(revision))).toThrow(
        "project Studio session is incomplete",
      );
    }
  });

  test("MotionContext accepts explicit success/error states and rejects implicit unsupported state", () => {
    expect(assertMotionContext({
      status: "error",
      protocolVersion: 1,
      generation: 3,
      input: "card.motion.tsx",
      diagnostics: [],
    }).status).toBe("error");
    expect(() => assertMotionContext({
      status: "error",
      protocolVersion: 1,
      generation: 3,
      input: "card.motion.tsx",
      diagnostics: [{ class: "resource", code: "failed", message: "failed", span: null }],
    })).toThrow("protocolVersion 1");
    const fixed = fixedPackage().render;
    const success = {
      status: "ok",
      protocolVersion: 1,
      generation: 3,
      input: "card.motion.tsx",
      artifactDigest: `sha256:${"a".repeat(64)}`,
      artifact: {
        formatVersion: 1,
        component: "Card",
        controls: {
          props: {},
          data: {},
          timing: {
            enterFrames: {},
            holdCycleFrames: {},
            exitFrames: {},
          },
          cues: {},
          assets: {},
          camera: { values: {} },
        },
      },
      preparedData: {},
      dataSource: null,
      timing: {},
      cueBindings: {
        beat: { type: "sourceRange", startFrame: 0, endFrame: 20, enterFrames: 2, exitFrames: 3 },
      },
      sourceMap: {
        version: 1,
        component: "Card",
        entry: "component.tsx",
        closureDigest: `sha256:${"c".repeat(64)}`,
        modules: [{
          path: "component.tsx",
          sourceDigest: `sha256:${"d".repeat(64)}`,
          normalizedAstDigest: `sha256:${"e".repeat(64)}`,
        }],
        nodes: [],
        exprs: [],
        objects: [],
      },
      assets: [],
      resourceLocators: [],
      shaders: [],
      durationFrames: 30,
      fps: { num: 30, den: 1 },
      viewport: { width: 640, height: 360 },
      diagnostics: [],
      runtimeBaseUrl: "/",
      runtimeAssets: {},
      fixedPackageManifestJson: fixed.fixedPackageManifestJson,
      timeline: canonicalTimeline(),
      timelineJson: JSON.stringify(canonicalTimeline()),
      resourceManifestJson: fixed.resourceManifestJson,
      resourceManifest: fixed.resourceManifest,
      verifiedBindingBundleJson: fixed.verifiedBindingBundleJson,
    };
    expect(assertMotionContext(success).status).toBe("ok");
    expect(() => assertMotionContext({ ...success, extraIdentity: "producer-private" })).toThrow(
      "successful MotionContext",
    );
    expect(() => assertMotionContext({ ...success, artifactDigest: "abc" })).toThrow("successful MotionContext");
    expect(() => assertMotionContext({ ...success, fonts: [] })).toThrow("successful MotionContext");
    const unsupportedControlsMirror = { ...success, controls: {} };
    expect(() => assertMotionContext(unsupportedControlsMirror)).toThrow("successful MotionContext");
    const missingArtifactControls = structuredClone(success);
    delete (missingArtifactControls.artifact as { controls?: unknown }).controls;
    expect(() => assertMotionContext(missingArtifactControls)).toThrow("successful MotionContext");
    const malformedArtifactControls = structuredClone(success);
    malformedArtifactControls.artifact.controls.timing = [] as never;
    expect(() => assertMotionContext(malformedArtifactControls)).toThrow("successful MotionContext");
    const mismatchedSourceMap = structuredClone(success);
    mismatchedSourceMap.sourceMap.component = "OtherCard";
    expect(() => assertMotionContext(mismatchedSourceMap)).toThrow("successful MotionContext");
    const missingClosureDigest = structuredClone(success);
    delete (missingClosureDigest.sourceMap as { closureDigest?: string }).closureDigest;
    expect(() => assertMotionContext(missingClosureDigest)).toThrow("successful MotionContext");
    const invalidClosureDigest = structuredClone(success);
    invalidClosureDigest.sourceMap.closureDigest = "sha256:closure";
    expect(() => assertMotionContext(invalidClosureDigest)).toThrow("successful MotionContext");
    const unsupportedSourceMap = structuredClone(success);
    Object.assign(unsupportedSourceMap.sourceMap, {
      closureSha256: unsupportedSourceMap.sourceMap.closureDigest,
      sourceSha256: unsupportedSourceMap.sourceMap.closureDigest,
    });
    expect(() => assertMotionContext(unsupportedSourceMap)).toThrow("successful MotionContext");
    const unsupportedModuleDigests = structuredClone(success);
    Object.assign(unsupportedModuleDigests.sourceMap.modules[0], {
      sourceSha256: unsupportedModuleDigests.sourceMap.modules[0].sourceDigest,
      normalizedAstSha256: unsupportedModuleDigests.sourceMap.modules[0].normalizedAstDigest,
    });
    expect(() => assertMotionContext(unsupportedModuleDigests)).toThrow("successful MotionContext");
    const malformedModuleDigest = structuredClone(success);
    malformedModuleDigest.sourceMap.modules[0].sourceDigest = "sha256:source";
    expect(() => assertMotionContext(malformedModuleDigest)).toThrow("successful MotionContext");
    const missingCueType = structuredClone(success);
    delete (missingCueType.cueBindings.beat as { type?: string }).type;
    expect(() => assertMotionContext(missingCueType)).toThrow("successful MotionContext");
    const unknownCueType = structuredClone(success);
    unknownCueType.cueBindings.beat.type = "unsupported";
    expect(() => assertMotionContext(unknownCueType)).toThrow("successful MotionContext");
    expect(() => assertMotionContext({ status: "ok", generation: 3 })).toThrow("protocolVersion 1");
  });
});
