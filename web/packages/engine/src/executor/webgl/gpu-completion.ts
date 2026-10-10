/** Drain the startup cushion once. Per-frame waits would serialize CPU/GPU execution. */
export async function waitForGpuCompletion(
  gl: WebGL2RenderingContext,
  timeoutMs = 15_000,
): Promise<void> {
  const sync = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0);
  if (!sync) throw new Error("GPU preview completion fence is unavailable");
  const started = performance.now();
  try {
    gl.flush();
    for (;;) {
      const status = gl.clientWaitSync(sync, 0, 0);
      if (status === gl.ALREADY_SIGNALED || status === gl.CONDITION_SATISFIED)
        return;
      if (status === gl.WAIT_FAILED)
        throw new Error("GPU preview completion failed");
      if (performance.now() - started >= timeoutMs)
        throw new Error("GPU preview completion timed out");
      await new Promise<void>((resolve) => setTimeout(resolve, 2));
    }
  } finally {
    gl.deleteSync(sync);
  }
}
