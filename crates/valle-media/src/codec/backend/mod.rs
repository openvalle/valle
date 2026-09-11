//! Native-pointer adapters for valle-media. Runtime discovery and ABI selection belong to valle-ffmpeg.
#![allow(clippy::duplicate_mod)]
use anyhow::Result;
pub(crate) use valle_ffmpeg::Version;
pub(crate) mod v7;
pub(crate) mod v8;
pub(crate) mod v9;

pub(crate) fn version() -> Result<Version> {
    Ok(valle_ffmpeg::init()?.version)
}
pub(crate) fn set_directory(path: std::path::PathBuf) -> Result<()> {
    Ok(valle_ffmpeg::set_directory(path)?)
}
pub(crate) fn update_log_level(level: i32) {
    valle_ffmpeg::set_log_level(level);
}

#[derive(Clone)]
pub(crate) enum Inner<A, B, C> {
    V7(A),
    V8(B),
    V9(C),
}

macro_rules! backend_type {
    ($name:ident, $module:ident) => {
        pub struct $name(
            pub(crate)  $crate::codec::backend::Inner<
                $crate::codec::backend::v7::$module::$name,
                $crate::codec::backend::v8::$module::$name,
                $crate::codec::backend::v9::$module::$name,
            >,
        );
        impl From<$crate::codec::backend::v7::$module::$name> for $name {
            fn from(value: $crate::codec::backend::v7::$module::$name) -> Self {
                Self($crate::codec::backend::Inner::V7(value))
            }
        }
        impl From<$crate::codec::backend::v8::$module::$name> for $name {
            fn from(value: $crate::codec::backend::v8::$module::$name) -> Self {
                Self($crate::codec::backend::Inner::V8(value))
            }
        }
        impl From<$crate::codec::backend::v9::$module::$name> for $name {
            fn from(value: $crate::codec::backend::v9::$module::$name) -> Self {
                Self($crate::codec::backend::Inner::V9(value))
            }
        }
    };
}
pub(crate) use backend_type;

macro_rules! owned_frame {
    ($name:ident, $module:ident) => {
        impl $name {
            pub(crate) fn into_v7(
                self,
            ) -> anyhow::Result<$crate::codec::backend::v7::$module::$name> {
                match self.0 {
                    $crate::codec::backend::Inner::V7(v) => Ok(v),
                    _ => anyhow::bail!("cannot pass a frame across FFmpeg ABIs"),
                }
            }
            pub(crate) fn into_v8(
                self,
            ) -> anyhow::Result<$crate::codec::backend::v8::$module::$name> {
                match self.0 {
                    $crate::codec::backend::Inner::V8(v) => Ok(v),
                    _ => anyhow::bail!("cannot pass a frame across FFmpeg ABIs"),
                }
            }
            pub(crate) fn into_v9(
                self,
            ) -> anyhow::Result<$crate::codec::backend::v9::$module::$name> {
                match self.0 {
                    $crate::codec::backend::Inner::V9(v) => Ok(v),
                    _ => anyhow::bail!("cannot pass a frame across FFmpeg ABIs"),
                }
            }
        }
    };
}
pub(crate) use owned_frame;

macro_rules! rational_conversion {
    ($version:ident) => {
        impl From<crate::codec::TimeBase> for valle_ffmpeg::backend::$version::Rational {
            fn from(v: crate::codec::TimeBase) -> Self {
                Self(v.0, v.1)
            }
        }
        impl From<valle_ffmpeg::backend::$version::Rational> for crate::codec::TimeBase {
            fn from(v: valle_ffmpeg::backend::$version::Rational) -> Self {
                Self(v.0, v.1)
            }
        }
    };
}
rational_conversion!(v7);
rational_conversion!(v8);
rational_conversion!(v9);

#[cfg(test)]
mod tests {
    // Each implementation is compiled for all three ABIs. Native tests must execute only
    // through the selected backend; run this binary with each VALLE_FFMPEG_DIR separately.
    // Pure Rust tests can stay in the shared implementation modules.
    macro_rules! native_test {
        ($(#[$attribute:meta])* $module:ident::$name:ident) => {
            $(#[$attribute])*
            #[test]
            fn $name() {
                match super::version().expect("native media tests require an FFmpeg installation") {
                    super::Version::V7 => super::v7::$module::tests::$name(),
                    super::Version::V8 => super::v8::$module::tests::$name(),
                    super::Version::V9 => super::v9::$module::tests::$name(),
                }
            }
        };
    }

    native_test!(alpha::cfr_writer_keeps_every_lossless_rgba_frame);
    native_test!(audio::forward_stream_uses_real_eof_instead_of_a_long_duration_hint);
    native_test!(audio::forward_stream_does_not_truncate_to_a_short_duration_hint);
    native_test!(alpha::timestamped_writer_keeps_a_single_nonzero_pts_frame);
    native_test!(decode::streaming_source_reports_and_decodes_square_pixel_sar_geometry);
    native_test!(flac::streaming_flac_roundtrip_preserves_length_and_pcm24_precision);
    native_test!(flac::strict_validation_rejects_truncated_and_trailing_data);
    native_test!(
        #[cfg(target_os = "macos")]
        #[ignore = "requires a real VideoToolbox device"]
        shared_frame::allocates_videotoolbox_bgra_frame
    );
}
