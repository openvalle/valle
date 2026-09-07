import "./console-app.ts";

const app = document.querySelector("valle-console-app");
if (!app) throw new Error("missing <valle-console-app>");
await app.updateComplete;
await import("./host.ts");
