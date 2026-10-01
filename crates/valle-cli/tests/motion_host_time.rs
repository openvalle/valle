use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
};

const SOURCE: &str = r##"export const composition={width:64,height:32,fps:4,duration:1};
export default function Main(ctx) {return <Scene><View className="absolute" style={{
    left:ctx.host.progress*40,width:12,height:12,top:ctx.seconds*8,
    backgroundColor:ctx.host.duration===2?"white":"red"}}/></Scene>; }"##;

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
            "--workers",
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
    let (left, top) = if frame == 0 { (0, 0) } else { (40, 6) };
    let at = (top * 64 + left) * 4;
    assert_eq!(&pixels[at..at + 4], &[255, 255, 255, 255]);
    let outside = (top * 64 + left + 12) * 4;
    assert_eq!(&pixels[outside..outside + 4], &[0, 0, 0, 0]);
}

#[test]
fn metal_host_clock_has_exact_endpoints_and_repeatable_output() {
    if std::env::var("VALLE_TEST_NATIVE_BACKEND").as_deref() != Ok("metal") {
        return;
    }
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
