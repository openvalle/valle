import { $ } from "bun";

const cwd = new URL("../tools/schema-gen/", import.meta.url).pathname;
const repo = new URL("../../", import.meta.url).pathname;
const web = new URL("../", import.meta.url).pathname;
await $`bun test scripts/generate-packed-abi.test.ts`.cwd(web);
await $`bun run scripts/generate-packed-abi.ts --check`.cwd(web);
await $`cargo run --quiet -p valle-timeline --features schema --bin valle-schema-gen -- --check`.cwd(repo);
await $`cargo test --quiet --manifest-path ${cwd}Cargo.toml`;
const first = await $`cargo run --quiet --manifest-path ${cwd}Cargo.toml -- --stdout`.text();
const second = await $`cargo run --quiet --manifest-path ${cwd}Cargo.toml -- --stdout`.text();

if (first !== second) {
  throw new Error("Rust → TypeScript generation is not byte deterministic");
}

await $`cargo run --quiet --manifest-path ${cwd}Cargo.toml -- --check`;

const drift = await $`cargo run --quiet --manifest-path ${cwd}Cargo.toml -- --drift-probe`.text();
if (drift === first || !drift.includes("changedField")) {
  throw new Error("negative protocol drift gate did not detect a simulated Rust field change");
}

console.log("protocol generation: deterministic, checked, drift-sensitive");
