use valle_engine::{
    compositor::reference::{
        BackdropSelector, EncodedPixel, LayerPipeline, PremulRgba32, ReferenceBlendMode,
        ReferenceImage, ReferenceLayer, decode_author_srgb, decode_input_pixel,
    },
    resource::{
        AuthorSrgbStraight, ColorDescription, ColorPrimaries, ColorRange, Extent2d, InputAlphaMode,
        MatrixCoefficients, SignalLuminance, TransferFunction, VisualInterpretation,
    },
};

#[test]
fn decoded_video_is_visible_to_motion_current_but_never_copied_into_local_source() {
    let extent = Extent2d::new(1, 1).unwrap();
    let video = decode_input_pixel(
        EncodedPixel::yuv([81, 90, 240], 255, 8).unwrap(),
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
    .unwrap();
    let first_motion = decode_author_srgb(AuthorSrgbStraight([0, 255, 0, 128])).unwrap();
    let later_motion = decode_author_srgb(AuthorSrgbStraight([0, 0, 255, 128])).unwrap();
    let video_image = ReferenceImage::solid(extent, video).unwrap();
    let first_image = ReferenceImage::solid(extent, first_motion).unwrap();
    let later_image = ReferenceImage::solid(extent, later_motion).unwrap();

    let mut layer = ReferenceLayer::root(video_image.clone()).unwrap();
    layer
        .draw_source(&first_image, ReferenceBlendMode::Normal)
        .unwrap();

    assert_eq!(layer.local_source(), &first_image);
    let current = layer.read_backdrop(BackdropSelector::Current).unwrap();
    let expected_current = first_motion.source_over(video).unwrap();
    assert!(
        current
            .image()
            .pixel(0, 0)
            .unwrap()
            .approx_eq(expected_current, 2e-6)
    );

    let scope = layer.capture_scope().unwrap();
    let frozen = layer
        .read_backdrop(BackdropSelector::ScopeEntry(scope))
        .unwrap();
    layer
        .draw_source(&later_image, ReferenceBlendMode::Normal)
        .unwrap();
    let frozen_again = layer
        .read_backdrop(BackdropSelector::ScopeEntry(scope))
        .unwrap();
    assert!(frozen.shares_storage(&frozen_again));
    assert_ne!(
        layer
            .read_backdrop(BackdropSelector::Current)
            .unwrap()
            .version(),
        frozen.version()
    );

    let final_image = layer.finish_over_entry(&LayerPipeline::default()).unwrap();
    let expected = later_motion
        .source_over(first_motion)
        .unwrap()
        .source_over(video)
        .unwrap();
    assert!(final_image.pixel(0, 0).unwrap().approx_eq(expected, 3e-6));
    assert_ne!(video, PremulRgba32::TRANSPARENT);
}
