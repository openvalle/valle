import { ProjectHost, loadStudioHost } from "./host.ts";

const [origin, projectId, baseRevisionText, timelinePath, intent] = Bun.argv.slice(2);
if (!origin || !projectId || !baseRevisionText || !timelinePath || !intent) {
  throw new Error("expected origin, project id, base revision, Timeline fixture path and intent");
}
const baseRevision = Number(baseRevisionText);
if (!Number.isSafeInteger(baseRevision) || baseRevision <= 0) {
  throw new Error("base revision must be a positive JavaScript-safe integer");
}

const fetcher = (input: RequestInfo | URL, init?: RequestInit) => (
  fetch(new URL(String(input), origin), init)
);
const host = await loadStudioHost(
  `?project=${encodeURIComponent(projectId)}`,
  fetcher,
  () => ({ addEventListener: () => undefined, close: () => undefined }),
);
if (!(host instanceof ProjectHost) || !host.loadTimeline || !host.saveTimeline) {
  throw new Error("expected the production ProjectHost adapter");
}

const snapshot = await host.loadTimeline();
const timeline = JSON.parse(await Bun.file(timelinePath).text());
const response = await host.saveTimeline({
  baseRevision,
  timeline,
  intent,
});
const reloaded = await host.loadTimeline();

console.log(JSON.stringify({
  boot: host.boot,
  snapshot,
  response,
  reloaded,
}));
