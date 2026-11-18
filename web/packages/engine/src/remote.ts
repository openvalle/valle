// Seekable media over HTTP. Only servers that ignore Range need a local copy.
export const READ_WINDOW = 2 * 1024 * 1024;

type Local = { file: File; remove: () => Promise<void> };

async function save(response: Response, signal: AbortSignal): Promise<Local> {
  const body = response.body!;
  try {
    const root = await navigator.storage.getDirectory();
    const dir = await root.getDirectoryHandle("valle-inputs", { create: true });
    const name = crypto.randomUUID();
    const handle = await dir.getFileHandle(name, { create: true });
    const remove = () => dir.removeEntry(name);
    try {
      await body.pipeTo(await handle.createWritable(), { signal });
      return { file: await handle.getFile(), remove };
    } catch (error) {
      await remove();
      throw error;
    }
  } catch (error) {
    await body.cancel().catch(() => {});
    if (signal.aborted) throw signal.reason;
    throw new Error(`the server ignored Range and the media could not be saved to OPFS: ${error}`);
  }
}

export class Remote {
  #local = new Map<string, Promise<Local>>();
  #lifetime = new AbortController();

  constructor(private request: typeof fetch) {}

  fetch: typeof fetch = async (input, init) => {
    const req = new Request(input, init);
    const range = /^bytes=(\d+)-(\d*)$/.exec(req.headers.get("Range") ?? "");
    if (!range) return this.request(input, init);
    const start = Number(range[1]);
    const end = range[2] ? Number(range[2]) : start + READ_WINDOW - 1;
    let copy = this.#local.get(req.url);
    if (!copy) {
      const headers = new Headers(req.headers);
      headers.set("Range", `bytes=${start}-${end}`);
      const response = await this.request(input, { ...init, headers });
      if (response.status === 206 && !/\/\d+$/.test(response.headers.get("Content-Range") ?? "")) {
        await response.body?.cancel();
        throw new Error("the media server must expose Content-Range with the full file size");
      }
      if (response.status !== 200 || !response.body) return response;
      // The demuxer's bounded cache can retain a complete small, unencoded response by itself.
      const length = response.headers.get("Content-Length");
      if (
        length &&
        /^\d+$/.test(length) &&
        Number(length) <= READ_WINDOW &&
        !response.headers.has("Content-Encoding")
      ) {
        return response;
      }
      copy = this.#local.get(req.url);
      if (copy) {
        await response.body.cancel();
      } else {
        copy = save(response, AbortSignal.any([req.signal, this.#lifetime.signal]));
        this.#local.set(req.url, copy);
        void copy.catch(() => this.#local.delete(req.url));
      }
    }
    const { file } = await copy;
    req.signal.throwIfAborted();
    if (start >= file.size) {
      return new Response(null, {
        status: 416,
        headers: { "Content-Range": `bytes */${file.size}` },
      });
    }
    const last = Math.min(end, file.size - 1);
    return new Response(file.slice(start, last + 1), {
      status: 206,
      headers: { "Content-Range": `bytes ${start}-${last}/${file.size}` },
    });
  };

  abort(): void {
    this.#lifetime.abort();
  }

  async dispose(): Promise<void> {
    this.abort();
    await Promise.all(
      [...this.#local.values()].map(async (copy) => (await copy.catch(() => undefined))?.remove()),
    );
    this.#local.clear();
  }
}
