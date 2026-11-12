# Releasing Valle

A release is one version of the executables, the browser engine on npm and the skill.

1. **Write the version** in `Cargo.toml`, `web/packages/engine/package.json`,
   `web/apps/studio/package.json` and `skills/valle/SKILL.md`, whose text also names the minor
   version it is written for. Run `cargo check` so `Cargo.lock` follows. A test of `xtask`
   fails while they differ, so CI is red on the commit, not on the tag.
2. **Tag the commit** `vX.Y.Z` once CI passes on main, and push the tag. CI builds, signs and
   notarizes the packages and drafts a GitHub release with them, the npm package and
   `SHA256SUMS`.
3. **Read the draft, write its notes and publish it.** Publishing puts the browser engine on
   npm, and runs the install scripts on every system they are for: each must install the
   version just published.
