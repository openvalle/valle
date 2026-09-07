//! Pure prepare-time computations for scales, projection, graph layout, and noise. Keep visual
//! themes and domain models outside this module. Route transcendental operations through shared
//! deterministic math because results become content-addressed artifact constants.

/// Author-facing names installed by the sandbox wrappers, distinct from internal operator names.
/// The compiler uses this list for prepare-only builtin diagnostics.
pub const AUTHOR_SURFACE: &[&str] = &[
    "ticks",
    "niceDomain",
    "extent",
    "extentOrDefault",
    "scaleLinear",
    "scaleBand",
    "scalePoint",
    "stack",
    "geoProject",
    "geoPath",
    "graphLayout",
    "placeLabels",
    "seededRandom",
    "noise1d",
    "noise2d",
];

pub mod bridge;
pub mod geo;
pub mod geo_path;
pub mod graph;
pub mod label;
pub mod noise;
pub mod scale;
pub mod stack;
