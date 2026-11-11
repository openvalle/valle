import { svg } from "lit";

const paths = {
  play: "m8 5 11 7-11 7Z",
  pause: "M8 5v14M16 5v14",
  back: "M6 5v14m12-14L8 12l10 7Z",
  next: "M18 5v14M6 5l10 7-10 7Z",
  undo: "m8 5-5 5 5 5M3 10h10a7 7 0 0 1 7 7",
  redo: "m16 5 5 5-5 5m5-5H11a7 7 0 0 0-7 7",
  file: "M14 3H5v18h14V8Zm0 0v5h5M8 12h8m-8 4h6",
  folder: "M3 6h7l2 2h9v12H3Zm0 0V4h7l2 2",
  image: "M3 3h18v18H3Zm0 14 6-6 5 5 3-3 4 4M15 7h.01",
  film: "M4 3h16v18H4Zm0 5h16M4 16h16M8 3v18M16 3v18",
  music: "M9 18V5l11-2v13M9 9l11-2M9 18c0 4-7 4-7 0s7-4 7 0m11-2c0 4-7 4-7 0s7-4 7 0",
  history: "M3 11a9 9 0 1 1 2 7M3 4v7h7m2-4v5l3 2",
  export: "M12 15V3m-4 4 4-4 4 4M4 13v7h16v-7",
  close: "m6 6 12 12M6 18 18 6",
  search: "M16 16 21 21M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0",
  focus: "M8 3H3v5m13-5h5v5M3 16v5h5m13-5v5h-5",
  panel: "M3 4h18v16H3ZM15 4v16",
  loop: "m17 2 4 4-4 4M3 11V9a3 3 0 0 1 3-3h15M7 22l-4-4 4-4m14-1v2a3 3 0 0 1-3 3H3",
  volume: "m11 4-6 5H2v6h3l6 5Zm4 4a6 6 0 0 1 0 8m3-11a10 10 0 0 1 0 14",
  muted: "m11 4-6 5H2v6h3l6 5Zm5 5 6 6m-6 0 6-6",
  chevron: "m8 10 4 4 4-4",
  check: "m4 12 5 5L20 6",
  save: "M4 3h14l3 3v15H3V3Zm3 0v6h10V3M7 21v-8h10v8",
  code: "m8 6-6 6 6 6m8-12 6 6-6 6m-3-15-2 18",
  eye: "M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12Zm10 3a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z",
  eyeOff:
    "m3 3 18 18M10.6 5.1Q11.3 5 12 5c6 0 10 7 10 7a17 17 0 0 1-3.2 3.9M6.5 6.6C3.7 8.5 2 12 2 12s4 7 10 7a9.7 9.7 0 0 0 4.4-1.1M9.9 9.9a3 3 0 0 0 4.2 4.2",
  lock: "M6 11h12v10H6Zm2 0V7a4 4 0 0 1 8 0v4",
  unlock: "M6 11h12v10H6Zm2 0V7a4 4 0 0 1 7.5-2",
  text: "M5 7V4h14v3M12 4v16m-3 0h6",
  sliders: "M4 6h10m4 0h2M4 12h4m4 0h8M4 18h10m4 0h2M14 4v4M8 10v4m6 2v4",
  layers: "m12 3 9 5-9 5-9-5Zm-9 9 9 5 9-5m-18 4 9 5 9-5",
  square: "M5 5h14v14H5Z",
  spark: "m12 3 2.2 6.8L21 12l-6.8 2.2L12 21l-2.2-6.8L3 12l6.8-2.2Z",
  plus: "M12 5v14M5 12h14",
  magnet: "M5 4h4v8a3 3 0 0 0 6 0V4h4v8a7 7 0 0 1-14 0Zm0 4h4m6 0h4",
} as const;

export type IconName = keyof typeof paths;

export function icon(name: IconName, size = 16) {
  return svg`<svg viewBox="0 0 24 24" width=${size} height=${size} fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d=${paths[name]} /></svg>`;
}
