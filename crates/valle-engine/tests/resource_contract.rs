use valle_engine::{
    frame::{RenderQuality, RenderSpec},
    resource::{
        AuthorSrgbStraight, ColorDescription, ColorPrimaries, ColorRange, ContentDigest, Dither,
        Extent2d, ExternalHandleId, ExternalPixelLayout, ExternalResourceDesc, FontFallbackChain,
        GamutMap, InputAlphaMode, LogicalTextureDesc, LuminanceNits, MatrixCoefficients,
        MediaDescriptor, NormalizedCrop, OutputAlphaMode, OutputBackground, OutputBitDepth,
        OutputColorEncoding, OutputSpec, PixelOrientation, ResourceContractError,
        ResourceInterpretation, ResourceKey, ResourceRequest, ResourceRequestSet, ResourceSample,
        SampleAspectRatio, SemanticAsset, SemanticAssetKind, SemanticFont, SemanticStructure,
        SignalLuminance, SnapshotBuilder, StructureDescriptor, TextureFormat, TextureUsage,
        ToneMap, TransferFunction, UnitFraction, VisualInterpretation, WorkingAlphaMode,
        WorkingColorSpace,
    },
};
use valle_timeline::RationalTime;

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_hex(&byte.to_string().repeat(64)).unwrap()
}

fn rec709_video(width: u32, height: u32) -> MediaDescriptor {
    MediaDescriptor::video(
        width,
        height,
        RationalTime::new(2, 1).unwrap(),
        VisualInterpretation::new(
            ColorDescription {
                primaries: ColorPrimaries::Rec709,
                transfer: TransferFunction::Rec709,
                matrix: MatrixCoefficients::Bt709,
                range: ColorRange::Limited,
            },
            SignalLuminance::SDR_100,
            InputAlphaMode::Opaque,
        ),
    )
    .unwrap()
}

#[test]
fn semantic_snapshot_freezes_media_font_fallback_and_structure() {
    let asset = SemanticAsset::new(
        "video",
        SemanticAssetKind::Video,
        digest('a'),
        rec709_video(1920, 1080),
    );
    let brand = SemanticFont::new("Brand", digest('b'), 0);
    let fallback = SemanticFont::new("Fallback", digest('c'), 2);
    let structure = SemanticStructure::new(
        "component://card",
        digest('d'),
        StructureDescriptor::Motion {
            topology_digest: digest('e'),
            bounds: Extent2d::new(640, 360).unwrap(),
        },
    );
    let left = SnapshotBuilder::new()
        .asset(asset.clone())
        .font(brand.clone())
        .font(fallback.clone())
        .fallback_chain(FontFallbackChain::new("Brand", ["Fallback"]))
        .structure(structure.clone())
        .finish()
        .unwrap();
    let right = SnapshotBuilder::new()
        .structure(structure)
        .font(fallback)
        .font(brand)
        .asset(asset)
        .fallback_chain(FontFallbackChain::new("Brand", ["Fallback"]))
        .finish()
        .unwrap();
    assert_eq!(
        left.assets().collect::<Vec<_>>(),
        right.assets().collect::<Vec<_>>()
    );
    assert_eq!(
        left.fonts().collect::<Vec<_>>(),
        right.fonts().collect::<Vec<_>>()
    );
    assert_eq!(
        left.structures().collect::<Vec<_>>(),
        right.structures().collect::<Vec<_>>()
    );
    assert_eq!(
        left.resolved_font_chain("Brand")
            .unwrap()
            .iter()
            .map(|font| (font.family.as_str(), font.face_index))
            .collect::<Vec<_>>(),
        [("Brand", 0), ("Fallback", 2)]
    );

    let wrong_kind = SnapshotBuilder::new()
        .asset(SemanticAsset::new(
            "image",
            SemanticAssetKind::Image,
            digest('f'),
            rec709_video(10, 10),
        ))
        .finish();
    assert!(wrong_kind.is_err());
}

#[test]
fn resource_key_changes_when_visual_interpretation_changes() {
    let base = VisualInterpretation::new(
        ColorDescription::SRGB,
        SignalLuminance::SDR_100,
        InputAlphaMode::StraightCoverage,
    );
    let crop = NormalizedCrop::new(
        UnitFraction::new(1, 10).unwrap(),
        UnitFraction::ZERO,
        UnitFraction::ONE,
        UnitFraction::ONE,
    )
    .unwrap();
    let interpretations = [
        base,
        base.with_orientation(PixelOrientation::Rotate90),
        base.with_sample_aspect_ratio(SampleAspectRatio::new(4, 3).unwrap()),
        base.with_crop(crop),
        VisualInterpretation::new(
            ColorDescription::LINEAR_REC2020,
            SignalLuminance::SDR_100,
            base.alpha,
        ),
        VisualInterpretation::new(
            ColorDescription::SRGB,
            SignalLuminance::SDR_100,
            InputAlphaMode::PremultipliedCoverage,
        ),
        VisualInterpretation::new(
            ColorDescription::SRGB,
            SignalLuminance::new(
                LuminanceNits::new(203).unwrap(),
                LuminanceNits::new(1000).unwrap(),
            )
            .unwrap(),
            InputAlphaMode::StraightCoverage,
        ),
    ];
    let keys: Vec<_> = interpretations
        .into_iter()
        .map(|interpretation| {
            ResourceKey::new(
                digest('a'),
                ResourceInterpretation::Visual { interpretation },
            )
        })
        .collect();
    for (index, left) in keys.iter().enumerate() {
        for right in &keys[index + 1..] {
            assert_ne!(left, right);
        }
    }
}

fn visual_request(handle: u32, orientation: PixelOrientation) -> ResourceRequest {
    let interpretation = VisualInterpretation::new(
        ColorDescription::SRGB,
        SignalLuminance::SDR_100,
        InputAlphaMode::StraightCoverage,
    )
    .with_orientation(orientation);
    ResourceRequest::new(
        ExternalHandleId::new(handle).unwrap(),
        ResourceKey::new(
            digest('a'),
            ResourceInterpretation::Visual { interpretation },
        ),
        ResourceSample::SourceTime(RationalTime::new(1, 24).unwrap()),
        ExternalResourceDesc::VisualFrame {
            extent: Extent2d::new(1920, 1080).unwrap(),
            pixel_layout: ExternalPixelLayout::Nv12,
        },
    )
    .unwrap()
}

#[test]
fn execution_requests_are_pure_canonical_data_and_wire_validation_is_closed() {
    let first = visual_request(1, PixelOrientation::Identity);
    let second = visual_request(2, PixelOrientation::Rotate90);
    let left = ResourceRequestSet::try_from_requests([second.clone(), first.clone()]).unwrap();
    let right = ResourceRequestSet::try_from_requests([first.clone(), second.clone()]).unwrap();
    assert_eq!(
        serde_json::to_vec(&left).unwrap(),
        serde_json::to_vec(&right).unwrap()
    );
    assert_eq!(left.requests()[0].handle().get(), 1);
    assert!(serde_json::from_str::<ExternalHandleId>("0").is_err());

    let mut value = serde_json::to_value(first).unwrap();
    value["expected"] = serde_json::json!({"kind":"fontBytes"});
    assert!(serde_json::from_value::<ResourceRequest>(value).is_err());
    assert!(matches!(
        ResourceRequestSet::try_from_requests([
            visual_request(1, PixelOrientation::Identity),
            visual_request(1, PixelOrientation::Rotate90),
        ]),
        Err(ResourceContractError::ConflictingHandle { .. })
    ));
}

#[test]
fn logical_textures_have_one_working_color_and_alpha_contract() {
    let texture = LogicalTextureDesc::new(
        Extent2d::new(1280, 720).unwrap(),
        TextureFormat::Rgba16Float,
        [TextureUsage::ColorAttachment, TextureUsage::Sampled],
        1,
    )
    .unwrap();
    assert_eq!(texture.working_space, WorkingColorSpace::LinearRec2020D65);
    assert_eq!(texture.alpha, WorkingAlphaMode::PremultipliedCoverage);
    assert!(
        LogicalTextureDesc::new(
            Extent2d::new(1, 1).unwrap(),
            TextureFormat::Rgba32Float,
            [TextureUsage::Sampled],
            3,
        )
        .is_err()
    );
}

#[test]
fn render_spec_carries_the_complete_output_contract() {
    let output = OutputSpec::new(
        OutputColorEncoding::DISPLAY_P3,
        OutputAlphaMode::StraightCoverage,
        OutputBackground::AuthorSrgbStraight {
            color: AuthorSrgbStraight([12, 34, 56, 128]),
        },
        ToneMap::ReinhardLuminance,
        GamutMap::ChromaCompress,
        Dither::Triangular { seed: 42 },
        OutputBitDepth::Sixteen,
        SignalLuminance::new(
            LuminanceNits::new(203).unwrap(),
            LuminanceNits::new(1000).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let spec = RenderSpec::new(1920, 1080, RenderQuality::Final, output).unwrap();
    assert_eq!(spec.output(), output);
    assert_eq!(spec.output().reference_white().get(), 203);
    assert_eq!(spec.output().peak_luminance().get(), 1000);
    assert_eq!(
        spec.output().background().author_color().0,
        [12, 34, 56, 128]
    );
}
