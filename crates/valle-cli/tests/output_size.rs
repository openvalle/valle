//! The render command changes delivery pixels, not the scene's layout canvas.

use std::path::Path;
use std::process::Command;

use indoc::indoc;
use serde_json::Value;
use valle_native::media::{self, Decoding};

fn run(dir: &Path, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_valle"))
        .args(args)
        .arg("--json")
        .current_dir(dir)
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn resized_png_sequence_and_video_keep_the_logical_canvas() {
    if !std::env::var("VALLE_TEST_LAYERS")
        .unwrap_or_default()
        .split(',')
        .any(|v| v == "gpu")
    {
        return;
    }
    let dir = std::env::temp_dir().join(format!("valle-output-size-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("scene.tsx"),
        indoc! {r##"
            export const composition = { width: 64, height: 32, fps: 2, duration: 1 };
            export default function Scene() {
              const { width, height } = useComposition();
              return <View style={{ width, height, background: "#ff0000" }}>
                <View style={{ position: "absolute", left: width / 2, top: 0, width: width / 2, height, background: "#00ff00" }} />
              </View>;
            }
        "##},
    )
    .unwrap();
    for (size, out, png, dimensions) in [
        ("128x64", "large.png", "large.png", [128, 64]),
        ("32x16", "frames", "frames/frame-00001.png", [32, 16]),
        ("64x64", "square.png", "square.png", [64, 64]),
    ] {
        let result = run(
            &dir,
            &[
                "motion",
                "render",
                "scene.tsx",
                "--output-size",
                size,
                "-o",
                out,
            ],
        );
        assert_eq!(result["status"], "ok", "{result}");
        let decoder = png::Decoder::new(std::io::BufReader::new(
            std::fs::File::open(dir.join(png)).unwrap(),
        ));
        let mut reader = decoder.read_info().unwrap();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes).unwrap();
        assert_eq!([info.width, info.height], dimensions);
        let [w, h] = dimensions.map(|v| v as usize);
        let at = |x, y| &bytes[4 * (y * w + x)..4 * (y * w + x + 1)];
        assert_eq!(at(w / 4, h / 2), [255, 0, 0, 255]);
        assert_eq!(at(3 * w / 4, h / 2), [0, 255, 0, 255]);
        if w == h {
            assert_eq!(at(0, 0), [0, 0, 0, 0]);
        }
    }
    for (encoder, codec) in [("ffmpeg", "h264"), ("vt", "hevc")] {
        if encoder == "vt" && !cfg!(target_os = "macos") {
            continue;
        }
        let result = run(
            &dir,
            &[
                "motion",
                "render",
                "scene.tsx",
                "--output-size",
                "128x64",
                "--encoder",
                encoder,
                "--codec",
                codec,
                "-o",
                "video.mp4",
            ],
        );
        assert_eq!(result["status"], "ok", "{result}");
        assert_eq!(result["direct"], encoder == "vt");
        let reader = media::reader(&dir.join("video.mp4"), Some(Decoding::Software)).unwrap();
        assert_eq!(reader.size(), [128, 64]);
    }
    // A shader reading its children gets the requested density even beyond the old 4096 cap.
    std::fs::write(
        dir.join("shader.tsx"),
        indoc! {r#"
            export const composition = { width: 100, height: 1, fps: 1, duration: 1 };
            const code = `
            @group(2) @binding(1) var content: texture_2d<f32>;
            fn shade(input: ShaderInput) -> vec4f {
              return select(vec4f(1., 0., 0., 1.), vec4f(0., 1., 0., 1.), textureDimensions(content).x == 5000u);
            }`;
            export default function Scene() {
              return <ShaderLayer code={code} style={{ width: 100, height: 1 }}><View style={{width:100,height:1,background:"white"}} /></ShaderLayer>;
            }
        "#},
    )
    .unwrap();
    let result = run(
        &dir,
        &[
            "motion",
            "render",
            "shader.tsx",
            "--output-size",
            "5000x50",
            "-o",
            "shader.png",
        ],
    );
    assert_eq!(result["status"], "ok", "{result}");
    let decoder = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(dir.join("shader.png")).unwrap(),
    ));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut pixels).unwrap();
    assert_eq!(
        &pixels[4 * (25 * 5000 + 2500)..4 * (25 * 5000 + 2500) + 4],
        [0, 255, 0, 255]
    );
    for size in ["0x32", "7680x4321", "10000x10", "large", "32x17"] {
        let result = run(
            &dir,
            &[
                "motion",
                "render",
                "scene.tsx",
                "--output-size",
                size,
                "-o",
                "bad.mp4",
            ],
        );
        assert_eq!(result["status"], "error", "{size}: {result}");
        assert!(!dir.join("bad.mp4").exists());
    }
    std::fs::remove_dir_all(dir).unwrap();
}
