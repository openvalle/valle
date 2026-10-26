export {};

if (new URLSearchParams(location.search).has("scene3dprobe")) {
  await import("./scene3d-probe.ts");
} else {
  await import("./host.ts");
}
