// Console application assembly.
//
// Library and project management backed by server commands. Asset actions use `/library/execute`;
// project actions use `/execute`. Fetch and rerender after mutations so the server remains
// authoritative.

import { bridgeProjectAssetToLibrary } from "./asset-bridge.ts";

export {};

interface ConsoleConfig { token?: string }
interface ApiError { message?: string }
interface Evidence { field: string; snippet: string }
interface AssetRow {
  hash: string;
  kind: string;
  title?: string;
  original_name?: string;
  subkind?: string;
  duration_ms?: number;
  removed_at?: string;
  stale?: boolean;
  tags?: string[];
  id?: string;
  type?: string;
}
interface SearchItem {
  asset: string;
  kind: string;
  title?: string;
  unit: string;
  range?: [number, number];
  evidence?: Evidence[];
}
interface ProjectRow { id: string; name?: string; head?: number; duration?: number }
interface TrackSummary { id: string; kind: string; clips?: unknown[] }
interface LogEntry { seq: number; cmd: string; note?: string; by?: string; ts?: string }
interface AnalysisSummary { analyzer: string; version: string; items: number; assets?: number; cost_fen: number; finished_at: string }
interface Annotation { id?: string; start_ms?: number; end_ms?: number; text?: string }
type AssetMeta = Omit<AssetRow, "hash"> & {
  content_digest: string;
  size?: number;
  added_at?: string;
  probe?: { duration_ms?: number; width?: number; height?: number; fps?: number };
};
interface ApiData {
  assets?: AssetRow[];
  results?: SearchItem[];
  projects?: ProjectRow[];
  canvas?: { width?: number; height?: number; fps?: number; duration?: number };
  tracks?: TrackSummary[];
  head?: number;
  log?: LogEntry[];
  meta?: AssetMeta;
  analysis?: AnalysisSummary[];
  annotations?: Annotation[];
  by_kind?: Record<string, number>;
  analyzers?: AnalysisSummary[];
  removed?: number;
  stale?: number;
}
interface ApiReport { ok: boolean; data?: ApiData; error?: ApiError }
type Verb = { verb: string; [key: string]: unknown };
interface LibraryEvent { kind?: "progress" | "done"; line?: string; ok?: boolean; report?: ApiReport }

declare global {
  var valleConsoleConfig: ConsoleConfig | undefined;
}

function requiredElement<T extends HTMLElement = HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing required Console element #${id}`);
  return element as T;
}

const $ = requiredElement;
const params = new URLSearchParams(location.search);

function errorText(error: unknown): string {
  return error instanceof Error ? error.stack ?? error.message : String(error ?? "unknown error");
}

async function postFailure(message: unknown): Promise<void> {
  await fetch("/result", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ status: "error", message: String(message) }),
  });
}

function showError(message: unknown): void {
  $("errtext").textContent = String(message ?? "unknown error");
  $("errbox").hidden = false;
}

window.addEventListener("error", (event) => {
  const message = errorText(event.error ?? event.message);
  showError(message);
  postFailure(message).catch(() => {});
});
window.addEventListener("unhandledrejection", (event) => {
  const message = errorText(event.reason);
  showError(message);
  postFailure(message).catch(() => {});
});

let config: ConsoleConfig = {};

// Asset command transport. Network failures throw; command failures return the shared response
// envelope.
async function lib(verb: Verb): Promise<ApiReport> {
  const res = await fetch("/library/execute", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-valle-token": config.token ?? "",
    },
    body: JSON.stringify(verb),
  });
  if (!res.ok) throw new Error(`library endpoint ${res.status}`);
  return await res.json() as ApiReport;
}

// Panel switching.
const PANELS = { library: "panelLibrary", projects: "panelProjects" };

function activeTab(): string {
  return document.querySelector<HTMLElement>('.tab[aria-selected="true"]')?.dataset.tab ?? "library";
}

function selectTab(name: string): void {
  for (const btn of document.querySelectorAll<HTMLElement>(".tab")) {
    btn.setAttribute("aria-selected", btn.dataset.tab === name ? "true" : "false");
  }
  for (const [tab, panelId] of Object.entries(PANELS)) {
    $(panelId).hidden = tab !== name;
  }
}

// Rendering helpers.
const KIND_ICON: Record<string, string> = {
  video: "🎬",
  audio: "🎵",
  image: "🖼️",
  font: "🔤",
  lottie: "✨",
  component: "🧩",
  other: "📄",
};

function fmtDur(ms: number | null | undefined): string {
  if (ms == null) return "";
  const s = ms / 1000;
  const m = Math.floor(s / 60);
  return `${m}:${(s - m * 60).toFixed(1).padStart(4, "0")}`;
}

function contentDigestHex(wire: string): string {
  const match = /^sha256:([0-9a-f]{64})$/.exec(wire);
  if (!match) throw new TypeError("asset metadata contains an invalid content_digest");
  return match[1]!;
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string | null = null,
  text: string | null = null,
): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  if (className) n.className = className;
  if (text != null) n.textContent = text;
  return n;
}

/// Use the asset-kind icon when no thumbnail is available.
function thumbNode(hash: string, kind: string): HTMLElement {
  const box = el("div", "thumb");
  const img = document.createElement("img");
  img.loading = "lazy";
  img.src = `/library/thumb/${hash}`;
  img.onerror = () => {
    img.remove();
    box.appendChild(el("span", "thumb-icon", KIND_ICON[kind] ?? KIND_ICON.other));
  };
  box.appendChild(img);
  return box;
}

function assetCard(row: AssetRow): HTMLElement {
  const card = el("div", "card");
  card.dataset.hash = row.hash;
  card.appendChild(thumbNode(row.hash, row.kind));
  const title = row.title || row.original_name || row.hash.slice(0, 12);
  card.appendChild(el("div", "card-title", title));
  const meta = el("div", "card-meta");
  meta.appendChild(el("span", "chip", row.subkind || row.kind));
  if (row.duration_ms != null) meta.appendChild(el("span", null, fmtDur(row.duration_ms)));
  if (row.removed_at) meta.appendChild(el("span", "chip removed", "removed"));
  if (row.stale) meta.appendChild(el("span", "chip removed", "stale"));
  card.appendChild(meta);
  if (Array.isArray(row.tags) && row.tags.length) {
    const tags = el("div", "card-tags");
    for (const t of row.tags) tags.appendChild(el("span", "chip tag", t));
    card.appendChild(tags);
  }
  card.addEventListener("click", () => openDetail(row.hash).catch((e) => showError(e)));
  return card;
}

function searchCard(item: SearchItem): HTMLElement {
  const card = el("div", "card");
  card.dataset.hash = item.asset;
  card.appendChild(thumbNode(item.asset, item.kind));
  card.appendChild(el("div", "card-title", item.title || item.asset.slice(0, 12)));
  const meta = el("div", "card-meta");
  meta.appendChild(el("span", "chip", item.unit));
  if (Array.isArray(item.range) && item.range.length === 2) {
    meta.appendChild(el("span", null, `${item.range[0].toFixed(1)}–${item.range[1].toFixed(1)}s`));
  }
  card.appendChild(meta);
  const ev = item.evidence?.[0];
  if (ev) card.appendChild(el("div", "evidence", `${ev.field}: ${ev.snippet}`));
  card.addEventListener("click", () => openDetail(item.asset).catch((e) => showError(e)));
  return card;
}

// Library grid and search.
let libraryReachable = false;

async function loadGrid(): Promise<number> {
  const report = await lib({ verb: "list", removed: $<HTMLInputElement>("libRemoved").checked || undefined });
  if (!report.ok) throw new Error(report.error?.message ?? "list failed");
  const rows = report.data?.assets ?? [];
  const grid = $("libGrid");
  grid.replaceChildren(...rows.map(assetCard));
  $("libEmpty").hidden = rows.length > 0;
  return rows.length;
}

async function runSearch(query: string): Promise<number> {
  const report = await lib({ verb: "search", query, limit: 50 });
  if (!report.ok) {
    // Keep the existing grid when a query is invalid and display the error.
    $("libNote").textContent = report.error?.message ?? "search failed";
    $("libNote").hidden = false;
    return 0;
  }
  $("libNote").hidden = true;
  const results = report.data?.results ?? [];
  $("libGrid").replaceChildren(...results.map(searchCard));
  $("libEmpty").hidden = results.length > 0;
  return results.length;
}

let searchTimer: ReturnType<typeof setTimeout> | null = null;
function onSearchInput(): void {
  if (searchTimer != null) clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    const q = $<HTMLInputElement>("libSearch").value.trim();
    const run = q ? runSearch(q) : loadGrid();
    run.catch((e) => showError(e));
  }, 200);
}

// Project command transport.
async function proj(command: Verb, project?: string): Promise<ApiReport> {
  const body: { command: Verb; project?: string } = { command };
  if (project) body.project = project;
  const res = await fetch("/execute", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-valle-token": config.token ?? "",
    },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`execute endpoint ${res.status}`);
  return await res.json();
}

// Project list, creation, details, and asset-library integration.
let projectsReachable = false;

function fmtSecs(s: number | null | undefined): string {
  return s != null ? `${Number(s).toFixed(1)}s` : "";
}

async function loadProjects(): Promise<number> {
  const report = await proj({ verb: "list" });
  if (!report.ok) throw new Error(report.error?.message ?? "list projects failed");
  const rows = report.data?.projects ?? [];
  const list = $("projList");
  list.replaceChildren(...rows.map(projectRow));
  $("projEmpty").hidden = rows.length > 0;
  return rows.length;
}

function projectRow(p: ProjectRow): HTMLElement {
  const row = el("div", "proj-row");
  row.dataset.project = p.id;
  const head = el("div", "proj-head");
  head.appendChild(el("b", null, p.name || "(Untitled)"));
  head.appendChild(el("span", "pid", p.id));
  head.appendChild(el("span", "meta", `head ${p.head ?? "?"} · ${fmtSecs(p.duration)}`));
  head.appendChild(el("span", "spacer"));
  const detailBtn = el("button", null, "Details");
  head.appendChild(detailBtn);
  const open = el("a", null, "Open Studio");
  open.href = `/studio?project=${encodeURIComponent(p.id)}`;
  head.appendChild(open);
  row.appendChild(head);

  const detail = el("div", "proj-detail");
  detail.hidden = true;
  row.appendChild(detail);
  detailBtn.addEventListener("click", () => {
    if (detail.hidden) {
      renderProjectDetail(p.id, detail).catch((e) => showError(e));
      detail.hidden = false;
      detailBtn.textContent = "Collapse";
    } else {
      detail.hidden = true;
      detailBtn.textContent = "Details";
    }
  });
  return row;
}

/// Combine project details, history, and library links. A shortened project asset address must
/// uniquely match a full library digest before linking.
async function renderProjectDetail(id: string, box: HTMLElement): Promise<void> {
  const [d, l, pa, libList] = await Promise.all([
    proj({ verb: "describe" }, id),
    proj({ verb: "log", limit: 20 }, id),
    proj({ verb: "list-assets" }, id),
    libraryReachable ? lib({ verb: "list" }) : Promise.resolve<ApiReport>({ ok: false }),
  ]);
  if (!d.ok) throw new Error(d.error?.message ?? "describe failed");
  const libHashes: string[] = libList.ok ? (libList.data?.assets ?? []).map((a: AssetRow) => a.hash) : [];

  box.replaceChildren();
  const c = d.data?.canvas ?? {};
  box.appendChild(
    el("div", null, `Canvas ${c.width}×${c.height}@${c.fps} · Duration ${fmtSecs(c.duration)} · head ${d.data?.head}`),
  );

  const tracks = d.data?.tracks ?? [];
  if (tracks.length) {
    box.appendChild(el("h5", null, "Tracks"));
    const ul = el("ul");
    for (const t of tracks) {
      ul.appendChild(el("li", null, `${t.id} (${t.kind}) · ${(t.clips ?? []).length} clips`));
    }
    box.appendChild(ul);
  }

  const assets = pa.ok ? (pa.data?.assets ?? []) : [];
  if (assets.length) {
    box.appendChild(el("h5", null, "Assets and library links"));
    const ul = el("ul");
    for (const a of assets) {
      const li = el("li", null, `${a.id} · ${a.type ?? ""} `);
      const bridge = bridgeProjectAssetToLibrary(a.id, libHashes);
      if (bridge.status === "matched") {
        const link = el("span", "bridge", "In library →");
        link.addEventListener("click", () => {
          selectTab("library");
          openDetail(bridge.contentDigest).catch((e) => showError(e));
        });
        li.appendChild(link);
        li.dataset.bridge = bridge.contentDigest;
      } else if (bridge.status === "ambiguous") {
        const label = el("span", "nobridge ambiguous", `Ambiguous library match (${bridge.contentDigests.length} candidates)`);
        label.title = bridge.contentDigests.join("\n");
        li.appendChild(label);
      } else if (bridge.status === "unaddressable") {
        li.appendChild(el("span", "nobridge", "Project asset has no verifiable content prefix"));
      } else {
        li.appendChild(el("span", "nobridge", "Asset outside library"));
      }
      ul.appendChild(li);
    }
    box.appendChild(ul);
  }

  const log = l.ok ? (l.data?.log ?? []) : [];
  if (log.length) {
    box.appendChild(el("h5", null, "Edit history"));
    const ul = el("ul");
    for (const r of log) {
      ul.appendChild(
        el("li", null, `#${r.seq} ${r.cmd}${r.note ? `——${r.note}` : ""} · ${r.by ?? ""} · ${r.ts ?? ""}`),
      );
    }
    box.appendChild(ul);
  }
}

// Library statistics.
async function renderDash(): Promise<void> {
  const report = await lib({ verb: "describe" });
  if (!report.ok) return;
  const d = report.data ?? {};
  const kinds = Object.entries(d.by_kind ?? {})
    .map(([k, n]) => `${k} ${n}`)
    .join(" · ");
  const cov = (d.analyzers ?? [])
    .map((a: AnalysisSummary) => `${a.analyzer} ${a.assets}`)
    .join(" · ");
  const costFen = (d.analyzers ?? []).reduce((s: number, a: AnalysisSummary) => s + (a.cost_fen ?? 0), 0);
  const parts = [`${d.assets ?? 0} assets`];
  if (kinds) parts.push(kinds);
  if (d.removed) parts.push(`removed ${d.removed}`);
  if (d.stale) parts.push(`stale ${d.stale}`);
  if (d.annotations) parts.push(`Annotations ${d.annotations}`);
  if (cov) parts.push(`Analysis:${cov}`);
  if (costFen) parts.push(`Total cost CNY ${(costFen / 100).toFixed(2)}`);
  $("libDash").textContent = parts.join(" ｜ ");
  $("libDash").hidden = false;
}

// Asset details and mutations.
let drawerContentDigest: string | null = null;

/// Execute a mutation, then refresh the drawer, grid, and dashboard from the server.
async function act(verb: Verb): Promise<boolean> {
  const report = await lib(verb);
  if (!report.ok) {
    $("libNote").textContent = report.error?.message ?? "action failed";
    $("libNote").hidden = false;
    return false;
  }
  $("libNote").hidden = true;
  if (drawerContentDigest) await openDetail(drawerContentDigest);
  await loadGrid();
  renderDash().catch(() => {});
  return true;
}

async function openDetail(selector: string): Promise<string> {
  const report = await lib({ verb: "show", hash: selector });
  if (!report.ok) throw new Error(report.error?.message ?? "show failed");
  const { meta, analysis = [], annotations = [] } = report.data ?? {};
  if (!meta) throw new Error("show response did not include asset metadata");
  const contentDigest = meta.content_digest;
  const hash = contentDigestHex(contentDigest);
  drawerContentDigest = contentDigest;
  $("dTitle").textContent = meta.title || meta.original_name || hash.slice(0, 12);
  $<HTMLInputElement>("dTitleEdit").value = meta.title ?? "";
  $("dSubkindRow").hidden = meta.kind !== "audio";
  $<HTMLSelectElement>("dSubkind").value = meta.subkind ?? "";
  $("dRm").textContent = "Remove, retain annotations";
  $("dRm").dataset.armed = "";
  $<HTMLButtonElement>("dRm").disabled = !!meta.removed_at;

  // Preview video, audio, and images directly from their blob URLs.
  const preview = $("dPreview");
  preview.replaceChildren();
  const blobUrl = `/library/blob/${hash}`;
  if (!meta.removed_at) {
    if (meta.kind === "video") {
      const v = document.createElement("video");
      v.controls = true;
      v.src = blobUrl;
      preview.appendChild(v);
    } else if (meta.kind === "audio") {
      const a = document.createElement("audio");
      a.controls = true;
      a.src = blobUrl;
      preview.appendChild(a);
    } else if (meta.kind === "image") {
      const img = document.createElement("img");
      img.src = blobUrl;
      preview.appendChild(img);
    }
  }

  const dl = $("dMeta");
  dl.replaceChildren();
  const kv = (k: string, v: unknown): void => {
    if (v == null || v === "") return;
    dl.appendChild(el("dt", null, k));
    dl.appendChild(el("dd", null, String(v)));
  };
  kv("content digest", meta.content_digest);
  kv("kind", meta.subkind ? `${meta.kind}/${meta.subkind}` : meta.kind);
  kv("Size", meta.size != null ? `${(meta.size / 1024 / 1024).toFixed(2)} MB` : null);
  kv("Original name", meta.original_name);
  kv("Added", meta.added_at);
  kv("removed", meta.removed_at);
  const p = meta.probe ?? {};
  kv("Duration", p.duration_ms != null ? fmtDur(p.duration_ms) : null);
  kv("Dimensions", p.width && p.height ? `${p.width}×${p.height}` : null);
  kv("fps", p.fps);

  const tags = $("dTags");
  tags.replaceChildren(
    ...(meta.tags ?? []).map((t: string) => {
      const chip = el("span", "chip tag", t);
      const x = el("span", "chip-x", "×");
      x.title = "Remove tag";
      x.addEventListener("click", (e: MouseEvent) => {
        e.stopPropagation();
        act({ verb: "tag", hash: contentDigest, rm: [t] }).catch((err) => showError(err));
      });
      chip.appendChild(x);
      return chip;
    }),
  );

  $("dAnnotations").replaceChildren(
    ...annotations.map((a) => {
      const range =
        a.start_ms != null && a.end_ms != null
          ? `[${(a.start_ms / 1000).toFixed(1)}–${(a.end_ms / 1000).toFixed(1)}s] `
          : a.start_ms != null
            ? `[${(a.start_ms / 1000).toFixed(1)}s] `
            : "";
      const li = el("li", null, `${a.id ?? ""} ${range}${a.text ?? ""}`);
      if (a.id) {
        const x = el("button", "ann-rm", "×");
        x.title = "Delete annotation";
        x.addEventListener("click", () =>
          act({ verb: "annotate", hash: contentDigest, rm: a.id }).catch((err) => showError(err)),
        );
        li.appendChild(x);
      }
      return li;
    }),
  );
  $("dAnalysis").replaceChildren(
    ...analysis.map((s) =>
      el(
        "li",
        null,
        `${s.analyzer}@${s.version} · ${s.items} items · ${s.cost_fen > 0 ? `${(s.cost_fen / 100).toFixed(2)} CNY · ` : ""}${s.finished_at}`,
      ),
    ),
  );
  $("drawer").hidden = false;
  return contentDigest;
}

function closeDetail(): void {
  $("drawer").hidden = true;
  // Remove media elements to stop playback and release streams.
  $("dPreview").replaceChildren();
}

async function main(): Promise<void> {
  for (const btn of document.querySelectorAll<HTMLElement>(".tab")) {
    btn.addEventListener("click", () => selectTab(btn.dataset.tab ?? "library"));
  }
  $("dClose").addEventListener("click", closeDetail);
  window.addEventListener("keydown", (e) => {
    if (e.key === "Escape") closeDetail();
  });

  // Drawer mutations share the same execution and refresh path.
  $("dTitleSave").addEventListener("click", () => {
    const title = $<HTMLInputElement>("dTitleEdit").value.trim();
    if (drawerContentDigest && title)
      act({ verb: "edit", hash: drawerContentDigest, title }).catch((e) => showError(e));
  });
  $("dSubkindSave").addEventListener("click", () => {
    const subkind = $<HTMLSelectElement>("dSubkind").value;
    if (drawerContentDigest && subkind)
      act({ verb: "edit", hash: drawerContentDigest, subkind }).catch((e) => showError(e));
  });
  $("dTagAdd").addEventListener("click", () => {
    const t = $<HTMLInputElement>("dTagInput").value.trim();
    if (drawerContentDigest && t) {
      $<HTMLInputElement>("dTagInput").value = "";
      act({ verb: "tag", hash: drawerContentDigest, add: [t] }).catch((e) => showError(e));
    }
  });
  $("dAnnAdd").addEventListener("click", () => {
    const text = $<HTMLInputElement>("dAnnText").value.trim();
    if (!drawerContentDigest || !text) return;
    const verb: Verb = { verb: "annotate", hash: drawerContentDigest, text };
    // Empty time means asset scope; one number is a marker and two hyphen-separated numbers are a
    // range in seconds.
    const r = $<HTMLInputElement>("dAnnRange").value.trim();
    if (r) {
      const m = r.match(/^([\d.]+)\s*[-–]\s*([\d.]+)$/);
      if (m) verb.range = [Number(m[1]), Number(m[2])];
      else if (/^[\d.]+$/.test(r)) verb.at = Number(r);
    }
    $<HTMLInputElement>("dAnnText").value = "";
    $<HTMLInputElement>("dAnnRange").value = "";
    act(verb).catch((e) => showError(e));
  });
  // Start analysis in the background; SSE library events report progress and completion.
  $("dAnRun").addEventListener("click", async () => {
    if (!drawerContentDigest) return;
    const with_ = $<HTMLInputElement>("dAnWith").value.split(",").map((s: string) => s.trim()).filter(Boolean);
    if (!with_.length) return;
    const verb: Verb = { verb: "analyze", hashes: [drawerContentDigest], with: with_ };
    const budget = Number($<HTMLInputElement>("dAnBudget").value);
    if (budget > 0) verb.budget = budget;
    try {
      const report = await lib(verb);
      if (!report.ok) {
        $("dProgress").textContent = report.error?.message ?? "analyze failed";
        $("dProgress").hidden = false;
        return;
      }
      $("dProgress").textContent = "Analysis started…";
      $("dProgress").hidden = false;
    } catch (e) {
      showError(e);
    }
  });

  // Require a second in-page click to confirm removal.
  $("dRm").addEventListener("click", () => {
    if (!drawerContentDigest) return;
    if ($<HTMLButtonElement>("dRm").dataset.armed !== "1") {
      $("dRm").dataset.armed = "1";
      $("dRm").textContent = "Confirm removal? Files will be removed; annotations retained.";
      return;
    }
    const contentDigest = drawerContentDigest;
    closeDetail();
    drawerContentDigest = null;
    act({ verb: "rm", hash: contentDigest }).catch((e) => showError(e));
  });
  $("libSearch").addEventListener("input", onSearchInput);
  $("libRemoved").addEventListener("change", () => loadGrid().catch((e) => showError(e)));
  $("libRefresh").addEventListener("click", () => {
    $<HTMLInputElement>("libSearch").value = "";
    loadGrid().catch((e) => showError(e));
  });

  // Project actions.
  $("projCreate").addEventListener("click", async () => {
    const name = $<HTMLInputElement>("projName").value.trim();
    $<HTMLInputElement>("projName").value = "";
    try {
      const report = await proj({ verb: "create", name: name || undefined });
      if (!report.ok) {
        $("projNote").textContent = report.error?.message ?? "create failed";
        $("projNote").hidden = false;
        return;
      }
      $("projNote").hidden = true;
      await loadProjects();
    } catch (e) {
      showError(e);
    }
  });
  $("projRefresh").addEventListener("click", () => loadProjects().catch((e) => showError(e)));

  try {
    const raw: unknown = await (await fetch("/config.json")).json();
    config = typeof raw === "object" && raw !== null ? raw as ConsoleConfig : {};
  } catch {
    config = {};
  }
  globalThis.valleConsoleConfig = config;

  // Check endpoint availability and show an unavailable state for static preview hosts.
  try {
    await loadGrid();
    libraryReachable = true;
    renderDash().catch(() => {});
    // Receive background analysis progress and completion; EventSource handles reconnection.
    const es = new EventSource("/events");
    es.addEventListener("library", (e) => {
      let msg: LibraryEvent = {};
      try {
        msg = JSON.parse(e.data);
      } catch {
        return;
      }
      if (msg.kind === "progress") {
        $("dProgress").textContent = msg.line ?? "";
        $("dProgress").hidden = false;
      } else if (msg.kind === "done") {
        $("dProgress").textContent = msg.ok
          ? "Analysis complete"
          : `Analysis failed: ${msg.report?.error?.message ?? ""}`;
        $("dProgress").hidden = false;
        // Refresh the open drawer, grid, and dashboard after completion.
        (drawerContentDigest ? openDetail(drawerContentDigest) : Promise.resolve())
          .then(() => loadGrid())
          .then(() => renderDash())
          .catch(() => {});
      }
    });
  } catch {
    libraryReachable = false;
    $("libNote").textContent = "This host has no asset library endpoint. Manage assets with `valle assets`.";
    $("libNote").hidden = false;
  }

  try {
    await loadProjects();
    projectsReachable = true;
  } catch {
    projectsReachable = false;
    $("projNote").textContent = "This host has no project endpoint. Start Studio with `valle project --project <ID> studio`.";
    $("projNote").hidden = false;
  }

  $("loading").hidden = true;

  if (params.get("smoke") === "1") {
    await runSmoke();
  }
}

// Smoke probes for the shell and reachable backend data.
async function runSmoke(): Promise<void> {
  const tabs = document.querySelectorAll(".tab").length;
  const initialTab = activeTab();
  const libraryVisibleBefore = !$("panelLibrary").hidden;

  $("tabBtnProjects").dispatchEvent(new MouseEvent("click", { bubbles: true }));
  const afterSwitch = {
    activeTab: activeTab(),
    projectsVisible: !$("panelProjects").hidden,
    libraryHidden: $("panelLibrary").hidden,
  };
  $("tabBtnLibrary").dispatchEvent(new MouseEvent("click", { bubbles: true }));

  const probe: Record<string, unknown> = {
    tabs,
    initialTab,
    libraryVisibleBefore,
    afterSwitch,
    backTab: activeTab(),
    libraryReachable,
  };

  if (libraryReachable) {
    probe.grid = document.querySelectorAll("#libGrid .card").length;
    await renderDash();
    probe.dashShown = !$("libDash").hidden;
    // Exercise search through the actual debounced input event path.
    const smokeQuery = params.get("smokeQuery") ?? "park";
    $<HTMLInputElement>("libSearch").value = smokeQuery;
    $("libSearch").dispatchEvent(new Event("input", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 500));
    probe.searchHits = document.querySelectorAll("#libGrid .card").length;
    // Return to the grid and open the first asset.
    $<HTMLInputElement>("libSearch").value = "";
    await loadGrid();
    const first = document.querySelector<HTMLElement>("#libGrid .card");
    if (first?.dataset.hash) {
      probe.detailContentDigest = await openDetail(first.dataset.hash);
      probe.detailOpened = !$("drawer").hidden;

      // Exercise mutations through DOM events and verify refreshed server state.
      $<HTMLInputElement>("dTitleEdit").value = "smoke-renamed";
      $("dTitleSave").dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await new Promise((r) => setTimeout(r, 400));
      probe.titleAfterEdit = $("dTitle").textContent;

      $<HTMLInputElement>("dTagInput").value = "sample";
      $("dTagAdd").dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await new Promise((r) => setTimeout(r, 400));
      probe.tagsAfterAdd = [...document.querySelectorAll("#dTags .chip")].map(
        (c) => c.firstChild?.textContent ?? c.textContent,
      );

      $<HTMLInputElement>("dAnnText").value = "Transition at three seconds";
      $<HTMLInputElement>("dAnnRange").value = "2.5-4";
      $("dAnnAdd").dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await new Promise((r) => setTimeout(r, 400));
      probe.annotationsAfterAdd = document.querySelectorAll("#dAnnotations li").length;

      // The first click arms removal; the second removes the asset, which appears in the removed
      // view.
      $("dRm").dispatchEvent(new MouseEvent("click", { bubbles: true }));
      probe.rmArmed = $<HTMLButtonElement>("dRm").dataset.armed === "1";
      $("dRm").dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await new Promise((r) => setTimeout(r, 500));
      probe.gridAfterRm = document.querySelectorAll("#libGrid .card").length;
      $<HTMLInputElement>("libRemoved").checked = true;
      $("libRemoved").dispatchEvent(new Event("change", { bubbles: true }));
      await new Promise((r) => setTimeout(r, 400));
      probe.removedView = document.querySelectorAll("#libGrid .card").length;
      $<HTMLInputElement>("libRemoved").checked = false;
      closeDetail();
    }
  }

  // Exercise project listing, details, integration, and creation through DOM events.
  probe.projectsReachable = projectsReachable;
  if (projectsReachable) {
    $("tabBtnProjects").dispatchEvent(new MouseEvent("click", { bubbles: true }));
    probe.projects = document.querySelectorAll("#projList .proj-row").length;
    const firstRow = document.querySelector<HTMLElement>("#projList .proj-row");
    if (firstRow) {
      const detailButton = firstRow.querySelector<HTMLButtonElement>("button");
      if (!detailButton) throw new Error("project row has no detail button");
      detailButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await new Promise((r) => setTimeout(r, 500));
      const detail = firstRow.querySelector<HTMLElement>(".proj-detail");
      if (!detail) throw new Error("project row has no detail panel");
      probe.projectDetailShown = !detail.hidden && detail.textContent.includes("head");
      probe.projectLogShown = detail.textContent.includes("Edit history");
      probe.bridgeHits = detail.querySelectorAll("li[data-bridge]").length;
      probe.studioLink = firstRow.querySelector("a")?.getAttribute("href") ?? "";
    }
    $<HTMLInputElement>("projName").value = "smoke-created";
    $("projCreate").dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 500));
    probe.projectsAfterCreate = document.querySelectorAll("#projList .proj-row").length;
    $("tabBtnLibrary").dispatchEvent(new MouseEvent("click", { bubbles: true }));
  }

  await fetch("/result", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      status: "ok",
      implementation: "valle-console",
      userAgent: navigator.userAgent,
      stats: { console: probe },
      captures: [],
    }),
  });
}

main().catch(async (err) => {
  showError(errorText(err));
  await postFailure(errorText(err)).catch(() => {});
});
