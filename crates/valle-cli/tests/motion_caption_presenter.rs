use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Output},
};

const SOURCE: &str = r##"export const composition={width:160,height:48,fps:4,duration:1};
export const role=captionPresenter({intro:seconds(0.25),outro:seconds(0.25)});
export default function Words(ctx,props,data) {
  return <Scene>{data.runs.map((run,index)=><Text key={index} className="absolute" style={{
    left:data.region[0]+index*60,top:data.region[1],fontFamily:data.style.font,
    fontSize:data.style.fontSize,color:ctx.host.seconds>=run.start && ctx.host.seconds<run.end?"#facc15":"#ffffff"
  }}>{run.text}</Text>)}</Scene>;
}"##;
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
fn caption_words_use_the_host_clock_reserved_font_and_repeatable_frames() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("words.motion.tsx"), SOURCE).unwrap();
    let input = json!({"text":"Build story","runs":[{"text":"Build ","start":0,"end":1},{"text":"story","start":1,"end":1.75}],"style":{"font":"asset://caption","fontSize":20,"color":"#ffffff"},"region":[8,8,144,32],"align":"bottom-center"});
    std::fs::write(dir.path().join("caption.json"), input.to_string()).unwrap();
    let font = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/fonts/noto/NotoSans-Regular.ttf")
        .canonicalize()
        .unwrap();
    let asset = format!("caption={}", font.display());
    let backend = std::env::var("VALLE_TEST_NATIVE_BACKEND").unwrap_or_else(|_| "raster".into());
    let sequence = report(run(
        dir.path(),
        &[
            "motion",
            "render",
            "words.motion.tsx",
            "--data",
            "caption.json",
            "--asset",
            &asset,
            "--host-duration",
            "2",
            "--backend",
            &backend,
            "--frames",
            "0,3,5,7",
            "--workers",
            "2",
            "-o",
            "sequence-%d.png",
            "--json",
        ],
    ));
    assert_eq!(sequence["compilations"], 1);
    for (pass, frame) in [7, 3, 0, 5, 3, 7].into_iter().enumerate() {
        let name = format!("direct-{frame}-{pass}.png");
        report(run(
            dir.path(),
            &[
                "motion",
                "render",
                "words.motion.tsx",
                "--data",
                "caption.json",
                "--asset",
                &asset,
                "--host-duration",
                "2",
                "--backend",
                &backend,
                "--frame",
                &frame.to_string(),
                "-o",
                &name,
                "--json",
            ],
        ));
        assert_eq!(
            std::fs::read(dir.path().join(name)).unwrap(),
            std::fs::read(dir.path().join(format!("sequence-{frame}.png"))).unwrap()
        );
    }
    // The template source holds at frames 3 and 5; the active word must still change.
    assert_ne!(
        std::fs::read(dir.path().join("sequence-3.png")).unwrap(),
        std::fs::read(dir.path().join("sequence-5.png")).unwrap()
    );
    assert_ne!(
        std::fs::read(dir.path().join("sequence-5.png")).unwrap(),
        std::fs::read(dir.path().join("sequence-7.png")).unwrap()
    );
    for frame in [3, 5] {
        let file = std::fs::File::open(dir.path().join(format!("sequence-{frame}.png"))).unwrap();
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (160, 48));
        assert!(
            pixels
                .chunks_exact(4)
                .any(|pixel| pixel[0] > 200 && pixel[1] > 160 && pixel[2] < 80 && pixel[3] > 100),
            "highlighted word must have visible glyph pixels"
        );
    }
    let invalid = run(
        dir.path(),
        &[
            "motion",
            "check",
            "words.motion.tsx",
            "--data",
            "caption.json",
            "--asset",
            &asset,
            "--host-duration",
            "1.5",
            "--json",
        ],
    );
    assert!(!invalid.status.success());
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&invalid.stdout),
        String::from_utf8_lossy(&invalid.stderr)
    );
    assert!(diagnostic.contains("runs/1"), "{diagnostic}");
    let missing_font = run(
        dir.path(),
        &[
            "motion",
            "check",
            "words.motion.tsx",
            "--data",
            "caption.json",
            "--host-duration",
            "2",
            "--json",
        ],
    );
    assert!(!missing_font.status.success());
}
