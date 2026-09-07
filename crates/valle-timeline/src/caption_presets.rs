//! Closed Timeline caption-preset catalog.
//!
//! The public author wire and the compiler consume the same catalog. The three
//! author-facing enums are phase-specific, while [`CaptionPreset`] is the
//! compiler-facing union used to expand a selection into ordinary curves.

use serde::{Deserialize, Serialize};

use crate::RationalTime;

pub const CAPTION_PRESET_PACK_ID: &str = "valle.caption-presets";
pub const CAPTION_PRESET_COUNT: usize = 27;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptionPresetPhase {
    Enter,
    Display,
    Exit,
}

/// Stable catalog metadata for agent and Studio pickers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptionPresetDescriptor {
    pub id: &'static str,
    pub phase: CaptionPresetPhase,
    pub label: &'static str,
    pub summary: &'static str,
    pub recommended_duration: Option<RationalTime>,
    pub recommended_rate: Option<f64>,
}

macro_rules! caption_preset_catalog {
    (
        enter { $(($enter_union_variant:ident, $enter_variant:ident, $enter_name:literal, $enter_label:literal, $enter_summary:literal, $enter_duration_ms:literal)),+ $(,)? }
        display { $(($display_union_variant:ident, $display_variant:ident, $display_name:literal, $display_label:literal, $display_summary:literal)),+ $(,)? }
        exit { $(($exit_union_variant:ident, $exit_variant:ident, $exit_name:literal, $exit_label:literal, $exit_summary:literal, $exit_duration_ms:literal)),+ $(,)? }
    ) => {
        #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum EnterCaptionPresetName {
            $(#[serde(rename = $enter_name)] $enter_variant),+
        }

        impl EnterCaptionPresetName {
            pub const ALL: &'static [Self] = &[$(Self::$enter_variant),+];

            pub const fn name(self) -> &'static str {
                match self { $(Self::$enter_variant => $enter_name),+ }
            }
        }

        #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum DisplayCaptionPresetName {
            $(#[serde(rename = $display_name)] $display_variant),+
        }

        impl DisplayCaptionPresetName {
            pub const ALL: &'static [Self] = &[$(Self::$display_variant),+];

            pub const fn name(self) -> &'static str {
                match self { $(Self::$display_variant => $display_name),+ }
            }
        }

        #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum ExitCaptionPresetName {
            $(#[serde(rename = $exit_name)] $exit_variant),+
        }

        impl ExitCaptionPresetName {
            pub const ALL: &'static [Self] = &[$(Self::$exit_variant),+];

            pub const fn name(self) -> &'static str {
                match self { $(Self::$exit_variant => $exit_name),+ }
            }
        }

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum CaptionPreset {
            $($enter_union_variant,)+
            $($exit_union_variant,)+
            $($display_union_variant),+
        }

        impl CaptionPreset {
            pub const ALL: [Self; CAPTION_PRESET_COUNT] = [
                $(Self::$enter_union_variant,)+
                $(Self::$exit_union_variant,)+
                $(Self::$display_union_variant),+
            ];

            pub const fn id(self) -> &'static str {
                match self {
                    $(Self::$enter_union_variant => concat!("enter.", $enter_name),)+
                    $(Self::$display_union_variant => concat!("display.", $display_name),)+
                    $(Self::$exit_union_variant => concat!("exit.", $exit_name),)+
                }
            }

            pub const fn phase(self) -> CaptionPresetPhase {
                match self {
                    $(Self::$enter_union_variant => CaptionPresetPhase::Enter,)+
                    $(Self::$display_union_variant => CaptionPresetPhase::Display,)+
                    $(Self::$exit_union_variant => CaptionPresetPhase::Exit,)+
                }
            }

            pub fn from_id(id: &str) -> Option<Self> {
                match id {
                    $(concat!("enter.", $enter_name) => Some(Self::$enter_union_variant),)+
                    $(concat!("display.", $display_name) => Some(Self::$display_union_variant),)+
                    $(concat!("exit.", $exit_name) => Some(Self::$exit_union_variant),)+
                    _ => None,
                }
            }

            pub fn descriptor(self) -> CaptionPresetDescriptor {
                match self {
                    $(Self::$enter_union_variant => descriptor(
                        self.id(), CaptionPresetPhase::Enter, $enter_label, $enter_summary,
                        Some($enter_duration_ms),
                    ),)+
                    $(Self::$display_union_variant => descriptor(
                        self.id(), CaptionPresetPhase::Display, $display_label, $display_summary,
                        None,
                    ),)+
                    $(Self::$exit_union_variant => descriptor(
                        self.id(), CaptionPresetPhase::Exit, $exit_label, $exit_summary,
                        Some($exit_duration_ms),
                    ),)+
                }
            }
        }

        impl From<EnterCaptionPresetName> for CaptionPreset {
            fn from(value: EnterCaptionPresetName) -> Self {
                match value { $(EnterCaptionPresetName::$enter_variant => Self::$enter_union_variant),+ }
            }
        }

        impl From<DisplayCaptionPresetName> for CaptionPreset {
            fn from(value: DisplayCaptionPresetName) -> Self {
                match value { $(DisplayCaptionPresetName::$display_variant => Self::$display_union_variant),+ }
            }
        }

        impl From<ExitCaptionPresetName> for CaptionPreset {
            fn from(value: ExitCaptionPresetName) -> Self {
                match value { $(ExitCaptionPresetName::$exit_variant => Self::$exit_union_variant),+ }
            }
        }
    };
}

caption_preset_catalog! {
    enter {
        (EnterFade, Fade, "fade", "Fade In", "A clean opacity reveal with a soft arrival.", 200),
        (EnterSlideUp, SlideUp, "slide-up", "Slide Up", "Rises into place while becoming visible.", 280),
        (EnterSlideDown, SlideDown, "slide-down", "Slide Down", "Drops into place while becoming visible.", 280),
        (EnterSlideLeft, SlideLeft, "slide-left", "Slide Left", "Moves left into place while becoming visible.", 280),
        (EnterSlideRight, SlideRight, "slide-right", "Slide Right", "Moves right into place while becoming visible.", 280),
        (EnterPop, Pop, "pop", "Pop In", "A compact scale-up with one restrained overshoot.", 300),
        (EnterZoomIn, ZoomIn, "zoom-in", "Zoom In", "Approaches from a slightly smaller scale.", 260),
        (EnterZoomOut, ZoomOut, "zoom-out", "Zoom Out", "Settles from a slightly larger scale.", 260),
        (EnterFocus, Focus, "focus", "Focus In", "Sharpens from a subtle blur and scale offset.", 300),
        (EnterWipeRight, WipeRight, "wipe-right", "Wipe Right", "Reveals the caption from left to right.", 260),
        (EnterWipeLeft, WipeLeft, "wipe-left", "Wipe Left", "Reveals the caption from right to left.", 260)
    }
    display {
        (DisplayBreathe, Breathe, "breathe", "Breathe", "A slow, low-amplitude scale cycle."),
        (DisplayFloat, Float, "float", "Float", "A slow vertical drift that returns to rest."),
        (DisplaySway, Sway, "sway", "Sway", "A gentle rotational pendulum motion."),
        (DisplayPulse, Pulse, "pulse", "Pulse", "A quiet two-beat scale accent with a long rest."),
        (DisplayShake, Shake, "shake", "Shake", "A short damped shake followed by a clean rest.")
    }
    exit {
        (ExitFade, Fade, "fade", "Fade Out", "A clean opacity departure with a soft release.", 160),
        (ExitSlideUp, SlideUp, "slide-up", "Slide Up", "Leaves the frame moving upward.", 220),
        (ExitSlideDown, SlideDown, "slide-down", "Slide Down", "Leaves the frame moving downward.", 220),
        (ExitSlideLeft, SlideLeft, "slide-left", "Slide Left", "Leaves the frame moving left.", 220),
        (ExitSlideRight, SlideRight, "slide-right", "Slide Right", "Leaves the frame moving right.", 220),
        (ExitPop, Pop, "pop", "Pop Out", "Releases through one restrained scale accent.", 240),
        (ExitZoomIn, ZoomIn, "zoom-in", "Zoom In", "Moves closer as the caption disappears.", 200),
        (ExitZoomOut, ZoomOut, "zoom-out", "Zoom Out", "Moves farther away as the caption disappears.", 200),
        (ExitFocus, Focus, "focus", "Defocus", "Softens into blur with a subtle scale release.", 240),
        (ExitWipeRight, WipeRight, "wipe-right", "Wipe Right", "Conceals the caption toward the right.", 200),
        (ExitWipeLeft, WipeLeft, "wipe-left", "Wipe Left", "Conceals the caption toward the left.", 200)
    }
}

fn descriptor(
    id: &'static str,
    phase: CaptionPresetPhase,
    label: &'static str,
    summary: &'static str,
    duration_ms: Option<u32>,
) -> CaptionPresetDescriptor {
    CaptionPresetDescriptor {
        id,
        phase,
        label,
        summary,
        recommended_duration: duration_ms.map(|duration_ms| {
            RationalTime::new(i64::from(duration_ms), 1_000).expect("static duration")
        }),
        recommended_rate: (phase == CaptionPresetPhase::Display).then_some(1.0),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn phase_enums_and_union_are_one_closed_catalog() {
        assert_eq!(
            EnterCaptionPresetName::ALL.len()
                + DisplayCaptionPresetName::ALL.len()
                + ExitCaptionPresetName::ALL.len(),
            CAPTION_PRESET_COUNT
        );
        let ids = CaptionPreset::ALL
            .iter()
            .map(|preset| preset.id())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), CAPTION_PRESET_COUNT);
        for preset in CaptionPreset::ALL {
            assert_eq!(CaptionPreset::from_id(preset.id()), Some(preset));
        }
    }

    #[cfg(feature = "schema")]
    #[test]
    fn phase_schemas_enumerate_only_their_catalog_names() {
        fn names<T: schemars::JsonSchema>() -> BTreeSet<String> {
            let schema = serde_json::to_value(schemars::schema_for!(T)).unwrap();
            schema["enum"]
                .as_array()
                .expect("unit enum schema")
                .iter()
                .map(|name| name.as_str().unwrap().to_owned())
                .collect()
        }

        assert_eq!(
            names::<EnterCaptionPresetName>(),
            EnterCaptionPresetName::ALL
                .iter()
                .map(|preset| preset.name().to_owned())
                .collect()
        );
        assert_eq!(
            names::<DisplayCaptionPresetName>(),
            DisplayCaptionPresetName::ALL
                .iter()
                .map(|preset| preset.name().to_owned())
                .collect()
        );
        assert_eq!(
            names::<ExitCaptionPresetName>(),
            ExitCaptionPresetName::ALL
                .iter()
                .map(|preset| preset.name().to_owned())
                .collect()
        );
    }
}
