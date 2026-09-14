/** Shared linear icon set for Studio / Console / Preview. */
export const ICON_PATHS = {
  undo: "M9 5 4 10l5 5M4 10h10a6 6 0 0 1 0 12",
  redo: "m15 5 5 5-5 5M20 10H10a6 6 0 0 0 0 12",
  sliders: "M4 7h5m4 0h7M4 17h9m4 0h3M9 4v6m4-6v6m0 4v6m4-6v6",
  timeline: "M3 5h18M3 12h6m4 0h8M3 19h12M7 3v4m9 3v4m-6 3v4",
  motion: "m12 3 9 5-9 5-9-5 9-5Zm-9 9 9 5 9-5m-18 5 9 5 9-5",
  "chevron-down": "m6 9 6 6 6-6",
  "chevron-right": "m9 6 6 6-6 6",
  panel: "M3 4h18v16H3V4Zm12 0v16",
  "skip-forward": "M19 5v14M5 5l11 7-11 7V5Z",
  "skip-back": "M5 5v14M19 5 8 12l11 7V5Z",
  "step-back": "M6 6v12m13-12-9 6 9 6V6Z",
  "step-forward": "M18 6v12M5 6l9 6-9 6V6Z",
  play: "m8 5 11 7-11 7V5Z",
  pause: "M8 5v14M16 5v14",
  repeat: "m17 2 4 4-4 4M3 11V9a3 3 0 0 1 3-3h15M7 22l-4-4 4-4m14-1v2a3 3 0 0 1-3 3H3",
  "volume-off": "m11 5-6 4H2v6h3l6 4V5Zm5 4 5 6m0-6-5 6",
  volume: "m11 5-6 4H2v6h3l6 4V5Zm5 3a5 5 0 0 1 0 8m3-11a9 9 0 0 1 0 14",
  expand: "M8 3H3v5m13-5h5v5M3 16v5h5m13-5v5h-5",
  minus: "M5 12h14",
  plus: "M5 12h14m-7-7v14",
  close: "m6 6 12 12M18 6 6 18",
  film: "M4 4h16v16H4V4Zm3 0v16M17 4v16M4 9h3m-3 6h3m10-6h3m-3 6h3",
  music: "M9 18V5l11-2v13M9 8l11-2M9 18c0 3-6 4-6 1s6-4 6-1Zm11-2c0 3-6 4-6 1s6-4 6-1Z",
  text: "M4 5h16M12 5v15m-5 0h10",
  effect: "m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5L12 3Z",
  lock: "M6 10h12v11H6V10Zm3 0V6a3 3 0 0 1 6 0v4",
  alert: "m12 3 10 18H2L12 3Zm0 6v5m0 3v1",
  "arrow-left": "m10 5-7 7 7 7M3 12h18",
  copy: "M8 8h13v13H8V8ZM16 8V3H3v13h5",
  check: "m5 12 4 4L19 6",
  save: "M5 4h12l2 2v14H5V4Zm3 0v6h8V4M8 20v-6h8v6",
} as const;

export type IconName = keyof typeof ICON_PATHS;

export function iconSvg(name: IconName, className = "icon"): string {
  const path = ICON_PATHS[name] ?? ICON_PATHS.motion;
  return `<svg class="${className}" viewBox="0 0 24 24" aria-hidden="true"><path d="${path}"/></svg>`;
}

export function hydrateIcons(root: ParentNode = document): void {
  root.querySelectorAll<HTMLElement>("[data-icon]").forEach((element) => {
    const name = element.dataset.icon as IconName | undefined;
    if (!name) return;
    element.innerHTML = iconSvg(name);
  });
}
