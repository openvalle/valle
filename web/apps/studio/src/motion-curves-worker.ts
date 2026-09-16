import { sampleMotionProperties, type MotionPropertyRequest } from "valle-engine/compiler";

self.onmessage = async (event: MessageEvent<{
  artifact: Record<string, unknown>; request: MotionPropertyRequest; runtimeAssets: unknown; runtimeBaseUrl: string;
}>) => {
  try {
    const { artifact, request, runtimeAssets, runtimeBaseUrl } = event.data;
    self.postMessage({ samples: await sampleMotionProperties({ runtimeAssets, runtimeBaseUrl }, artifact, request) });
  } catch (error) {
    self.postMessage({ error: error instanceof Error ? error.message : String(error) });
  }
};
