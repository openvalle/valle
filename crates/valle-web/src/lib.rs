//! Valle's engine in the browser, which `cargo xtask build-web` builds for wasm32: `web` binds it
//! to the SDK's worker, and `yuv` turns the planes of the video frames it decodes into pixels.
//! Elsewhere only `yuv` is built, to be tested.

#[cfg(target_arch = "wasm32")]
mod web;
pub mod yuv;
