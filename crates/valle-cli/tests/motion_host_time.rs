//! Run Metal acceptance outside the sandbox with
//! `VALLE_TEST_NATIVE_BACKEND=metal cargo test -p valle-cli --test motion_host_time -- --include-ignored --test-threads=1`.

mod support;

use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
};

const SOURCE: &str = r##"export const composition={width:64,height:32,fps:4,duration:1};
export default function Main() { const time = useCurrentTime(); const comp = useComposition();return <Scene><View className="absolute" style={{
    left:time.clipProgress*40,width:12,height:12,top:time.seconds*8,
    backgroundColor:comp.clipDuration===2?"white":"red"}}/></Scene>; }"##;

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_valle"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}
fn report(output: Output) -> Value {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn host_duration_is_shared_by_check_review_and_exact_frame_delivery() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("host.motion.tsx"), SOURCE).unwrap();
    report(run(
        dir.path(),
        &[
            "motion",
            "check",
            "host.motion.tsx",
            "--host-duration",
            "2",
            "--frame",
            "7",
            "--json",
        ],
    ));
    let review = report(run(
        dir.path(),
        &[
            "motion",
            "review",
            "host.motion.tsx",
            "--host-duration",
            "2",
            "--json",
        ],
    ));
    assert_eq!(review["framesAnalyzed"], 8);
    let sequence = report(run(
        dir.path(),
        &[
            "motion",
            "render",
            "host.motion.tsx",
            "--host-duration",
            "2",
            "--backend",
            "raster",
            "--frames",
            "0,3,7",
            "-o",
            "sequence-%d.png",
            "--threads",
            "2",
            "--json",
        ],
    ));
    assert_eq!(sequence["compilations"], 1);
    for (pass, frame) in [7, 0, 3, 7].into_iter().enumerate() {
        let name = format!("direct-{frame}-{pass}.png");
        report(run(
            dir.path(),
            &[
                "motion",
                "render",
                "host.motion.tsx",
                "--host-duration",
                "2",
                "--backend",
                "raster",
                "--frame",
                &frame.to_string(),
                "-o",
                &name,
                "--json",
            ],
        ));
        assert_eq!(
            std::fs::read(dir.path().join(&name)).unwrap(),
            std::fs::read(dir.path().join(format!("sequence-{frame}.png"))).unwrap()
        );
    }
    for frame in [0, 7] {
        assert_host_endpoint(&dir.path().join(format!("sequence-{frame}.png")), frame);
    }
    for duration in ["0", "0.001"] {
        assert!(
            !run(
                dir.path(),
                &[
                    "motion",
                    "check",
                    "host.motion.tsx",
                    "--host-duration",
                    duration,
                    "--json"
                ]
            )
            .status
            .success()
        );
    }
}

fn assert_host_endpoint(path: &Path, frame: u32) {
    let mut reader = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((info.width, info.height), (64, 32));
    assert_eq!(info.color_type, png::ColorType::Rgba);
    // The last frame of the host shows the end of the scene, which it holds from 1 s.
    let (left, top) = if frame == 0 { (0, 0) } else { (40, 8) };
    let at = (top * 64 + left) * 4;
    assert_eq!(
        &pixels[at..at + 4],
        &support::srgb_delivery_pixel([255; 4], left, top)
    );
    let outside = (top * 64 + left + 12) * 4;
    assert_eq!(&pixels[outside..outside + 4], &[0, 0, 0, 0]);
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "requires a real Metal device; run outside the sandbox with --include-ignored"]
fn metal_host_clock_has_exact_endpoints_and_repeatable_output() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("host.motion.tsx"), SOURCE).unwrap();
    for frame in [0, 7] {
        let mut images = Vec::new();
        for pass in 0..2 {
            let name = format!("metal-{frame}-{pass}.png");
            report(run(
                dir.path(),
                &[
                    "motion",
                    "render",
                    "host.motion.tsx",
                    "--host-duration",
                    "2",
                    "--backend",
                    "metal",
                    "--frame",
                    &frame.to_string(),
                    "-o",
                    &name,
                    "--json",
                ],
            ));
            assert_host_endpoint(&dir.path().join(&name), frame);
            images.push(std::fs::read(dir.path().join(name)).unwrap());
        }
        assert_eq!(images[0], images[1]);
    }
}

#[test]
fn overlay_delivery_preserves_endpoints_and_uses_the_host_viewport() {
    let backend = if std::env::var("VALLE_TEST_NATIVE_BACKEND").as_deref() == Ok("metal") {
        "metal"
    } else {
        "raster"
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("overlay.motion.tsx"),r#"
export const composition={width:64,height:32,fps:4,duration:2};
export const role=overlay({intro:0.5,outro:0.5,hold:"once"});
export default function Main() { const time = useCurrentTime(); const comp = useComposition();return <Scene><View key="bar" className="absolute"
 style={{left:time.seconds*8,width:comp.width/4,height:comp.height/2,backgroundColor:"white"}}/></Scene>;}
"#).unwrap();
    for size in ["64x32", "32x64"] {
        let mut endpoints = Vec::new();
        for (duration, last) in [("1.5", "5"), ("3", "11")] {
            let review = report(run(
                dir.path(),
                &[
                    "motion",
                    "review",
                    "overlay.motion.tsx",
                    "--host-duration",
                    duration,
                    "--host-size",
                    size,
                    "--json",
                ],
            ));
            assert_eq!(review["framesAnalyzed"], last.parse::<u32>().unwrap() + 1);
            report(run(
                dir.path(),
                &[
                    "motion",
                    "check",
                    "overlay.motion.tsx",
                    "--host-duration",
                    duration,
                    "--host-size",
                    size,
                    "--frame",
                    last,
                    "--json",
                ],
            ));
            let mut images = Vec::new();
            for (pass, frame) in [last, "0", last].into_iter().enumerate() {
                let name = format!("{size}-{duration}-{frame}-{pass}.png");
                report(run(
                    dir.path(),
                    &[
                        "motion",
                        "render",
                        "overlay.motion.tsx",
                        "--host-duration",
                        duration,
                        "--host-size",
                        size,
                        "--backend",
                        backend,
                        "--frame",
                        frame,
                        "-o",
                        &name,
                        "--json",
                    ],
                ));
                let bytes = std::fs::read(dir.path().join(name)).unwrap();
                let mut reader = png::Decoder::new(std::io::Cursor::new(&bytes))
                    .read_info()
                    .unwrap();
                let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
                let info = reader.next_frame(&mut pixels).unwrap();
                let (width, height) = if size == "64x32" { (64, 32) } else { (32, 64) };
                assert_eq!((info.width, info.height), (width, height));
                // The last frame shows the end of the template: 2 s.
                let left = if frame == "0" { 0 } else { 16 };
                assert_eq!(
                    &pixels[left * 4..left * 4 + 4],
                    &support::srgb_delivery_pixel([255; 4], left, 0)
                );
                let outside = (height / 2 * width + left as u32) as usize * 4;
                assert_eq!(&pixels[outside..outside + 4], &[0, 0, 0, 0]);
                images.push(bytes);
            }
            assert_eq!(images[0], images[2]);
            endpoints.push(images);
        }
        assert_eq!(endpoints[0], endpoints[1]);
    }
    assert!(
        !run(
            dir.path(),
            &[
                "motion",
                "check",
                "overlay.motion.tsx",
                "--host-duration",
                "0.99",
                "--json"
            ]
        )
        .status
        .success()
    );
}

#[test]
fn an_outro_shorter_than_a_frame_warns_that_no_frame_shows_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let write = |outro: &str| {
        std::fs::write(
            dir.path().join("title.motion.tsx"),
            format!(
                r#"export const composition={{width:64,height:32,fps:4,duration:4}};
export const role=overlay({{intro:0,outro:{outro},hold:"once"}});
export default function Title() {{ const time = useCurrentTime(); return <Scene><View key="bar" className="absolute"
 style={{{{left:time.seconds*8,width:8,height:8,backgroundColor:"white"}}}}/></Scene>; }}
"#
            ),
        )
        .unwrap();
    };
    std::fs::write(
        dir.path().join("title.timeline.json"),
        serde_json::json!({"canvas":{"width":64,"height":32,"fps":4},"resources":{"title":"title.motion.tsx"},
            "tracks":{"visual":[{"clips":[{"kind":"motion","component":"title","start":0,"duration":3}]}]}})
        .to_string(),
    )
    .unwrap();
    let warned = |args: &[&str]| {
        let output = run(dir.path(), args);
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let warnings = report(output)["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|warning| warning["code"] == "unsampled-end")
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            stderr.contains("[warning:UnsampledEnd]"),
            !warnings.is_empty(),
            "{stderr}"
        );
        warnings
    };
    let check = [
        "motion",
        "check",
        "title.motion.tsx",
        "--host-duration",
        "3",
        "--json",
    ];
    let timeline = ["timeline", "check", "title.timeline.json", "--json"];
    // Four seconds of title in a clip of three cut into the outro where it starts. At 4 fps
    // the last frame of 3 seconds is at 2.75 s, before an outro of 0.1 s starts.
    write("0.1");
    let found = warned(&check);
    assert_eq!(found.len(), 1);
    let message = found[0]["message"].as_str().unwrap();
    assert!(message.contains("`outro: 0.25`"), "{message}");
    let found = warned(&timeline);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["path"], "/tracks/visual/0/clips/0");
    let message = found[0]["message"].as_str().unwrap();
    assert!(message.contains(" at 0–3 s"), "{message}");
    // An outro of a frame starts on the last frame, which steps to the end.
    write("0.25");
    assert!(warned(&check).is_empty());
    assert!(warned(&timeline).is_empty());
}
