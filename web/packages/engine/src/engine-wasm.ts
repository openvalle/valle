const modules = new Map<string, Promise<unknown>>();

/** One initialization promise per Engine module in this realm, shared by compiler and player. */
export function loadEngineWasm(moduleUrl: string, wasmUrl: string): Promise<unknown> {
  const key = JSON.stringify([moduleUrl, wasmUrl]);
  let loading = modules.get(key);
  if (!loading) {
    loading = (async () => {
      const module = await import(moduleUrl);
      await module.default({ module_or_path: wasmUrl });
      return module;
    })();
    modules.set(key, loading);
    void loading.catch(() => {
      if (modules.get(key) === loading) modules.delete(key);
    });
  }
  return loading;
}
