// What the page and the engine's worker say to each other: a request with an id, answered once.

/** A file a page gives in place of fetching a path: its bytes, or where to fetch them. */
export type Source = string | Blob | ArrayBuffer | Uint8Array;

/** What a scene's controls are bound to: props and data by name, and the files of resources. */
export type Inputs = {
  props: Record<string, unknown>;
  data: Record<string, unknown>;
  /** Each resource's files, as paths the worker reads. */
  resources: Record<string, string[]>;
};

export type Request =
  | {
      type: "start";
      canvas: OffscreenCanvas;
      /** The address relative paths are fetched from. */
      base: string;
      /** Where the built-in fonts are fetched from, by their file names, if not their own addresses. */
      fonts?: string;
    }
  | { type: "give"; files: Record<string, Source> }
  | { type: "open"; entry: string; inputs: Inputs }
  | { type: "frame"; index: number }
  | { type: "pixels" }
  | { type: "close" }
  | { type: "destroy" };

/** The composition and the controls of what is open. */
export type Opened = {
  width: number;
  height: number;
  /** Frames a second, as a whole number or a fraction such as `30000/1001`. */
  fps: string;
  frames: number;
  duration: number;
  loop: boolean;
  controls: Controls;
};

/** What a scene's controls take, as `defineProps`, `defineData` and `defineResources` declare them. */
export type Controls = {
  props: Record<string, Control>;
  data: Record<string, Control>;
  resources: Record<string, { kind: string; default?: string[] }>;
};

export type Control = {
  type: string;
  default?: unknown;
  required?: boolean;
  label?: string;
  min?: number;
  max?: number;
  step?: number;
  values?: unknown[];
};

/** Why a request failed: `no-webgpu`, a `file` that could not be fetched, or the `scene`'s error. */
export type Code = "no-webgpu" | "file" | "scene";

export type Reply = { id: number } & (
  { ok: true; value?: unknown } | { ok: false; code: Code; message: string }
);
