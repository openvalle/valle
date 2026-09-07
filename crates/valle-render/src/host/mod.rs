//! Native Product Compositor host.
//!
//! Scheduling and resource fulfillment live here; pixel semantics do not. Every caller—preview,
//! export, storyboard and CLI capture—uses [`FrameRunner`], while the sealed executor consumes only
//! plan/bindings/backend objects.

#[cfg(feature = "native")]
mod audio;
mod frame;
#[cfg(feature = "lottie")]
mod lottie;
#[cfg(feature = "native")]
mod pipeline;
#[cfg(feature = "native")]
mod project;
#[cfg(feature = "native")]
mod render;
#[cfg(feature = "native")]
mod resource;
mod schedule;
#[cfg(feature = "native")]
mod sink;

#[cfg(feature = "native")]
pub use audio::{AudioMixError, CompiledAudioMixer};
#[cfg(all(target_os = "macos", feature = "native"))]
pub(crate) use frame::SubmittedSharedFrame;
pub use frame::{
    FrameDelivery, FrameEvidence, FrameRenderError, FrameRunner, ResourceCacheFrameReport,
    ResourceFrameReport, ResourceFulfillment, ResourceProvider, Scene3dFrameReport,
};
#[cfg(feature = "native")]
pub use pipeline::{
    PipelineError, PipelineReport, ProgressCallback, RenderControl, render_schedule,
};
#[cfg(feature = "native")]
pub use project::NativeProject;
#[cfg(feature = "native")]
pub use render::{NativeRenderError, NativeRenderOptions, NativeRenderer, RenderSummary};
#[cfg(feature = "native")]
pub use resource::{
    NativeResourceCacheLimits, NativeResourceCatalog, NativeResourceError, NativeResourceProvider,
    NativeResourceSource, probe_image_extent,
};
pub use schedule::{FrameSchedule, FrameScheduleError};
#[cfg(feature = "native")]
pub use sink::{
    DeliveredFrame, DeliveredSharedFrame, FrameSink, Mp4Sink, PngSink, StoryboardSink, write_png,
};
