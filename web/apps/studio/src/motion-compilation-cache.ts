import type { CompiledMotion } from "valle-engine";
import { digestBytes } from "./audio-analysis-input.ts";

export interface MotionCompilationCacheOptions {
  url: string;
  token: string;
}

/** A disposable Host cache survives browser restarts and random Studio ports. */
export class MotionCompilationCache {
  readonly #options: MotionCompilationCacheOptions;
  readonly #compiler: Promise<string | null>;
  readonly #keys = new Map<string, Promise<string | null>>();

  constructor(options: MotionCompilationCacheOptions) {
    this.#options = options;
    this.#compiler = this.#fetch(options.url)
      .then(async (response) => {
        if (response.status !== 200) return null;
        const value = (await response.json()) as { compilerDigest?: unknown };
        return typeof value.compilerDigest === "string" &&
          /^sha256:[0-9a-f]{64}$/.test(value.compilerDigest)
          ? value.compilerDigest
          : null;
      })
      .catch(() => null);
  }

  #fetch(url: string, init: RequestInit = {}) {
    return fetch(url, {
      ...init,
      headers: { "x-valle-token": this.#options.token, ...init.headers },
    });
  }

  #key(inputs: string): Promise<string | null> {
    let key = this.#keys.get(inputs);
    if (!key) {
      key = this.#compiler.then((identity) =>
        identity ? hash(JSON.stringify([identity, inputs])) : null,
      );
      this.#keys.set(inputs, key);
      if (this.#keys.size > 128) this.#keys.delete(this.#keys.keys().next().value!);
    }
    return key;
  }

  async get(inputs: string): Promise<CompiledMotion | null> {
    try {
      const key = await this.#key(inputs);
      if (!key) return null;
      const response = await this.#fetch(`${this.#options.url}/${key}`);
      if (response.status !== 200) return null;
      const record = (await response.json()) as { digest?: unknown; compiled?: CompiledMotion };
      const compiled = record.compiled;
      if (
        !compiled ||
        !/^sha256:[0-9a-f]{64}$/.test(compiled.artifactDigest) ||
        !compiled.artifact ||
        typeof compiled.artifact !== "object" ||
        !compiled.sourceMap ||
        !Array.isArray(compiled.warnings) ||
        typeof compiled.normalizedSource !== "string" ||
        typeof compiled.normalizedAstDigest !== "string" ||
        typeof compiled.preparedDataDigest !== "string" ||
        record.digest !== (await hash(JSON.stringify(compiled)))
      )
        return null;
      return compiled;
    } catch {
      return null;
    }
  }

  async put(inputs: string, compiled: CompiledMotion): Promise<void> {
    try {
      const key = await this.#key(inputs);
      if (!key) return;
      const body = JSON.stringify({ digest: await hash(JSON.stringify(compiled)), compiled });
      if (body.length > 8 * 1024 * 1024) return;
      await this.#fetch(`${this.#options.url}/${key}`, { method: "POST", body });
    } catch {
      /* Compilation and admission work normally when the cache is unavailable. */
    }
  }
}

function hash(value: string): Promise<string> {
  return digestBytes(new TextEncoder().encode(value));
}
