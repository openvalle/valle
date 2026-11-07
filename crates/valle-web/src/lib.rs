//! Valle's engine in the browser, which `cargo xtask build-web` builds for wasm32: `web` binds it
//! to the SDK's worker. Video color conversion lives in the shared engine.

#[cfg(target_arch = "wasm32")]
mod web;
pub use valle_engine::image::yuv;
