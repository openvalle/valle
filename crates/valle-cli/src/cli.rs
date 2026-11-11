//! The command line as clap reads it: each command, its options, and their help. Values are
//! checked here where they need nothing but their text; what needs the scene or a file is
//! checked by the command.

use std::path::PathBuf;

use clap::{Args as Group, Parser, Subcommand, ValueEnum};
use valle_native::media::{Codec, Decoding, Encoding};
use valle_timeline::query::Filter;
use valle_timeline::time::Rate;

use crate::Failure;
use crate::args::{self, Args, Frames};

#[derive(Parser)]
#[command(
    name = "valle",
    version,
    about = "The video engine for AI agents: Motion graphics written in TSX and edits of footage, \
             sound and captions written in JSON, checked, rendered and previewed on your machine",
    arg_required_else_help = true,
    after_help = "New here? Read `valle docs start`. Run `valle <command> --help` for a \
                  command's options."
)]
pub struct Cli {
    /// Print the result as JSON
    #[arg(long, global = true)]
    pub json: bool,
    /// Print what it does as it goes, a line of JSON each, then the result as JSON
    #[arg(long, global = true)]
    pub events: bool,
    /// The folder of FFmpeg's shared libraries, in place of `VALLE_FFMPEG_DIR` and settings.json
    #[arg(long, global = true, value_name = "FOLDER")]
    pub ffmpeg_dir: Option<std::path::PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// A Motion scene: a *.motion.tsx file
    #[command(subcommand, arg_required_else_help = true)]
    Motion(Motion),
    /// A Timeline document: a *.timeline.json file
    #[command(subcommand, arg_required_else_help = true)]
    Timeline(Timeline),
    /// A project folder: project.json, its timelines, files and snapshots
    Project(Project),
    /// The guides: how to write scenes and timelines, and how the commands behave
    ///
    /// Without a topic, lists them. A guide is long; --section prints one of its parts.
    Docs {
        /// The guide
        #[arg(value_parser = clap::builder::PossibleValuesParser::new(crate::docs::topics()))]
        topic: Option<String>,
        /// One section of the guide, by its name or heading
        #[arg(long, requires = "topic")]
        section: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum Motion {
    /// Compile the scene and check every frame without drawing
    #[command(
        after_help = "Example:\n  valle motion check title.motion.tsx --props '{\"title\":\"Hi\"}'"
    )]
    Check {
        /// The scene: a .motion.tsx file
        #[arg(value_name = "SCENE")]
        file: String,
        #[command(flatten)]
        bind: Bindings,
        #[command(flatten)]
        frames: FrameChoice,
    },
    /// Draw frames into a .png, a folder of them, or a .mp4 or .mov video
    #[command(after_help = "Examples:\n  \
        valle motion render title.motion.tsx --frame 45 -o frame.png\n  \
        valle motion render title.motion.tsx --frames 0,30,60 --storyboard -o sheet.png\n  \
        valle motion render title.motion.tsx -o title.mp4")]
    Render {
        /// The scene: a .motion.tsx file
        #[arg(value_name = "SCENE")]
        file: String,
        #[command(flatten)]
        bind: Bindings,
        #[command(flatten)]
        render: Output,
    },
    /// Open Studio in the browser on the scene
    ///
    /// Studio is served from 127.0.0.1 until stopped.
    Studio {
        /// The scene: a .motion.tsx file
        #[arg(value_name = "SCENE")]
        file: PathBuf,
        #[command(flatten)]
        serve: Serve,
    },
}

#[derive(Subcommand)]
pub enum Timeline {
    /// Check the document, its clips' sources and every frame
    #[command(after_help = "Example:\n  valle timeline check edit.timeline.json\n\n\
        How to write one: valle docs timeline")]
    Check {
        /// The timeline: a .timeline.json file
        #[arg(value_name = "TIMELINE")]
        file: String,
        #[command(flatten)]
        frames: FrameChoice,
    },
    /// Draw frames into a .png, a folder or a video with the sound, or the sound into a .wav
    #[command(after_help = "Examples:\n  \
        valle timeline render edit.timeline.json --frame 60 -o frame.png\n  \
        valle timeline render edit.timeline.json -o edit.mp4\n  \
        valle timeline render edit.timeline.json -o mix.wav")]
    Render {
        /// The timeline: a .timeline.json file
        #[arg(value_name = "TIMELINE")]
        file: String,
        #[command(flatten)]
        render: Output,
    },
    /// The clips the filters pick, with their timecodes, lines and anchors
    ///
    /// Every filter given holds. Each clip comes with its program and source timecodes, its
    /// line in the file, what it is anchored to and the clips anchored to it.
    Query {
        /// The timeline: a .timeline.json file
        #[arg(value_name = "TIMELINE")]
        file: String,
        #[command(flatten)]
        filter: Filters,
    },
    /// Edit it as Studio does: set, move, trim, delete or add a clip
    #[command(after_help = EDIT_EXAMPLES, after_long_help = EDITING)]
    Edit {
        /// The timeline: a .timeline.json file
        #[arg(value_name = "TIMELINE")]
        file: String,
        /// The edit, as JSON or @file: { "op": "move", "clip": "intro", "start": 2 }
        #[arg(long, value_name = "JSON|@FILE")]
        op: String,
    },
    /// Open Studio in the browser on the document
    ///
    /// Studio is served from 127.0.0.1 until stopped.
    Studio {
        /// The timeline: a .timeline.json file
        #[arg(value_name = "TIMELINE")]
        file: PathBuf,
        #[command(flatten)]
        serve: Serve,
    },
}

const EDIT_EXAMPLES: &str = "\
Examples:
  valle timeline edit edit.timeline.json --op '{\"op\":\"move\",\"clip\":\"shot\",\"start\":3}'
  valle timeline edit edit.timeline.json --op '{\"op\":\"trim\",\"clip\":\"shot\",\"duration\":4}'
  valle timeline edit edit.timeline.json --op @edit.json

Run with --help for the editing conventions.";

const EDITING: &str = "\
Examples:
  valle timeline edit edit.timeline.json --op '{\"op\":\"move\",\"clip\":\"shot\",\"start\":3}'
  valle timeline edit edit.timeline.json --op '{\"op\":\"trim\",\"clip\":\"shot\",\"duration\":4}'
  valle timeline edit edit.timeline.json --op @edit.json

Editing conventions:
  clip / track             an id or JSON path, as query returns in path / trackAt
  move.index               the destination array index from 0; out of range is an error;
                           transitions are removed when their neighbors separate
  move.track               another track of the clip's kind: the end of a sequence, or among
                           placed clips at start, else where the clip was
  set id                   updates every anchor that names the clip
  set track                with track in place of clip: a track's own field, such as enabled
  delete                   also removes adjacent transitions and clips anchored to it
  add                      appends to a track; tracks.<kind>[n] makes a track when n is the
                           number of tracks of that kind (0 for the first)
  start / head / duration  seconds, snapped to the timeline's frames

An edit checks its sources' bounds before writing; one that fails leaves the file unchanged.
An empty timeline can be edited and queried, not rendered.";

#[derive(Group)]
#[command(arg_required_else_help = true)]
pub struct Project {
    /// The project in this folder, not the one the current folder is in
    #[arg(short = 'C', value_name = "FOLDER", global = true)]
    pub folder: Option<PathBuf>,
    #[command(subcommand)]
    pub command: ProjectCommand,
}

#[derive(Subcommand)]
pub enum ProjectCommand {
    /// A new project: project.json, main.timeline.json and its folders
    Create {
        /// Where to make it
        folder: PathBuf,
        /// The canvas's width in pixels
        #[arg(long, default_value_t = 1920, value_parser = clap::value_parser!(u32).range(1..))]
        width: u32,
        /// The canvas's height in pixels
        #[arg(long, default_value_t = 1080, value_parser = clap::value_parser!(u32).range(1..))]
        height: u32,
        /// Frames a second, such as 30 or 30000/1001
        #[arg(long, default_value = "30")]
        fps: String,
    },
    /// Its timelines, resources, files, snapshots and last render
    ///
    /// Files out of the project and missing ones are listed too.
    Show,
    /// Copy files in, or link them, and bind each in the entry timeline's resources
    ///
    /// Components go into components/, everything else into media/; --link leaves them where
    /// they are.
    Add {
        /// Files or folders
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Link them where they are instead of copying
        #[arg(long)]
        link: bool,
    },
    /// Check a timeline, its files and its components' folders
    Check {
        /// The timeline; the entry one unless given
        timeline: Option<String>,
        /// Also plan every frame
        #[arg(long)]
        cost: bool,
    },
    /// Save a snapshot of the sources
    Save {
        /// What changed
        #[arg(short, long)]
        message: Option<String>,
    },
    /// The snapshots
    History,
    /// Put a snapshot's sources back, saving what they replace as a new one
    Restore {
        /// The snapshot, from 1
        #[arg(value_parser = clap::value_parser!(u64).range(1..))]
        snapshot: u64,
    },
    /// Render a timeline, by default a video into renders/
    ///
    /// A full render, every frame into a video, first saves a snapshot, and records beside it
    /// what it read and made.
    Render {
        /// The timeline; the entry one unless given
        timeline: Option<String>,
        /// A .png, a folder, a .mp4 or .mov video, or a .wav; a new file in renders/ unless
        /// given
        #[arg(short, long, value_name = "OUT")]
        out: Option<String>,
        #[command(flatten)]
        options: RenderOptions,
    },
    /// Open Studio in the browser on the project
    ///
    /// Studio is served from 127.0.0.1 until stopped.
    Studio {
        /// The project's folder; the current one's unless given
        folder: Option<PathBuf>,
        #[command(flatten)]
        serve: Serve,
    },
}

/// Values bound to a scene's controls.
#[derive(Group)]
#[command(next_help_heading = "Bindings")]
pub struct Bindings {
    /// Values for the scene's controls.props, as JSON or @file: '{"title":"Hi"}'
    #[arg(long, value_name = "JSON|@FILE")]
    props: Option<String>,
    /// Values for the scene's controls.data, as JSON or @file
    #[arg(long, value_name = "JSON|@FILE")]
    data: Option<String>,
    /// A file for one of controls.resources; give a font's faces once each
    #[arg(long, value_name = "NAME=PATH", value_parser = pair)]
    resource: Vec<(String, String)>,
    /// A picture the scene reads as input://NAME, in place of a timeline's
    #[arg(long, value_name = "NAME=PATH", value_parser = pair)]
    input: Vec<(String, String)>,
    /// The caption a captionPresenter() scene shows: '{"text":"Hello"}' or with runs
    #[arg(long, value_name = "JSON|@FILE")]
    caption: Option<String>,
}

/// The frames a command works on.
#[derive(Group)]
#[group(multiple = false)]
pub struct FrameChoice {
    /// The one frame, from 0; a .png shows frame 0 unless given
    #[arg(long, value_name = "N", value_parser = frame, allow_hyphen_values = true)]
    frame: Option<i64>,
    /// A list (3,0,3) or a range with both ends (0..59), keeping order and duplicates
    #[arg(long, value_name = "A,B,…|A..B", value_parser = frames, allow_hyphen_values = true)]
    frames: Option<Frames>,
}

/// Where a render goes, and how.
#[derive(Group)]
#[command(next_help_heading = "Render")]
pub struct Output {
    /// A .png (one frame), a folder (a sequence), a .mp4 or .mov video, or, for a timeline, a
    /// .wav of its sound
    #[arg(short, long, value_name = "OUT")]
    out: String,
    #[command(flatten)]
    options: RenderOptions,
}

#[derive(Group)]
#[command(next_help_heading = "Render")]
pub struct RenderOptions {
    #[command(flatten)]
    frames: FrameChoice,
    /// Draw a PNG storyboard: sample 12 frames including the first and last by default
    #[arg(long)]
    storyboard: bool,
    /// Uniformly sample up to N frames, including the first and last
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..),
        requires = "storyboard", conflicts_with_all = ["step", "frame", "frames"])]
    samples: Option<u32>,
    /// Take frame 0, N, 2N, ... for the storyboard
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..),
        requires = "storyboard", conflicts_with_all = ["samples", "frame", "frames"])]
    step: Option<u32>,
    /// Frames per PNG page (12 by default); multiple pages use NAME-001.png, NAME-002.png, ...
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..), requires = "storyboard")]
    per_page: Option<u32>,
    /// Output pixels, without relayout, up to 7680x4320
    #[arg(long, value_name = "WxH", value_parser = size, allow_hyphen_values = true)]
    output_size: Option<[u32; 2]>,
    /// Output frames a second, such as 30, 60 or 30000/1001, without changing duration
    #[arg(long, value_name = "FPS", value_parser = Rate::parse, allow_hyphen_values = true)]
    fps: Option<Rate>,
    /// A video's codec; h264 for a .mp4 and prores (4444, with alpha) for a .mov unless given
    #[arg(long)]
    codec: Option<CodecName>,
    /// The video encoder; the hardware's where it has the codec unless given
    #[arg(long)]
    encoder: Option<Coder>,
    /// The decoder of videos the scene shows; the hardware's where it opens the file unless
    /// given
    #[arg(long)]
    decoder: Option<Coder>,
    /// A video's bits a second, such as 8M, in place of its quality
    #[arg(long, value_name = "BITS", value_parser = bits, allow_hyphen_values = true)]
    bitrate: Option<u64>,
}

#[derive(Clone, Copy, ValueEnum)]
enum CodecName {
    H264,
    Hevc,
    Prores,
}

#[derive(Clone, Copy, ValueEnum)]
enum Coder {
    /// The hardware's (VideoToolbox)
    #[value(name = "vt", alias = "hardware")]
    Hardware,
    /// FFmpeg's own, on the CPU: x264, x265, prores_ks
    #[value(name = "ffmpeg", alias = "software")]
    Software,
}

/// What `timeline query` picks.
#[derive(Group)]
#[command(next_help_heading = "Filters")]
pub struct Filters {
    /// The clip by id or JSON path, such as tracks.visual[0].items[2]
    #[arg(long, value_name = "ID|PATH")]
    clip: Option<String>,
    /// The track by id or JSON path, such as tracks.visual[0]
    #[arg(long, value_name = "ID|PATH")]
    track: Option<String>,
    /// Its JSON array index from 0, gaps and transitions counted
    #[arg(long, value_name = "N", value_parser = index, allow_hyphen_values = true)]
    index: Option<usize>,
    /// The clips on at that time, with the source timecode there
    #[arg(long, value_name = "SECONDS|TIMECODE")]
    at: Option<String>,
    /// The clips that carry that marker, or that the timeline's marker of that name is on
    #[arg(long, value_name = "NAME")]
    marker: Option<String>,
    /// Metadata: a dotted path from the top, or a key at any depth
    #[arg(long, value_name = "KEY=VALUE", value_parser = meta)]
    meta: Vec<(String, String)>,
}

/// How Studio is served.
#[derive(Group)]
pub struct Serve {
    /// The port to listen on; 0 lets the system pick
    #[arg(long, default_value_t = 9527)]
    pub port: u16,
    /// Serve Studio from a web/ build instead of the embedded one
    #[arg(long, value_name = "FOLDER")]
    pub web_assets_dir: Option<PathBuf>,
}

impl Bindings {
    fn apply(self, args: &mut Args) -> Result<(), Failure> {
        let i = &mut args.inputs;
        if let Some(props) = self.props {
            i.props = args::object(&props, "--props")?;
        }
        if let Some(data) = self.data {
            i.data = args::object(&data, "--data")?;
        }
        if let Some(caption) = self.caption {
            i.caption = Some(args::json(&caption, "--caption")?);
        }
        for (name, path) in self.resource {
            i.resources.entry(name).or_default().push(path);
        }
        i.pictures.extend(self.input);
        Ok(())
    }
}

impl FrameChoice {
    fn get(self) -> Option<Frames> {
        (self.frame.map(|f| Frames::List(vec![f]))).or(self.frames)
    }
}

impl RenderOptions {
    fn apply(self, args: &mut Args) {
        args.frames = self.frames.get();
        args.storyboard = self.storyboard;
        args.samples = self.samples;
        args.step = self.step;
        args.per_page = self.per_page;
        args.output_size = self.output_size;
        args.fps = self.fps;
        args.codec = self.codec.map(|c| match c {
            CodecName::H264 => Codec::H264,
            CodecName::Hevc => Codec::Hevc,
            CodecName::Prores => Codec::ProRes4444,
        });
        args.encoder = self.encoder.map(|c| match c {
            Coder::Hardware => Encoding::Hardware,
            Coder::Software => Encoding::Software,
        });
        args.decoder = self.decoder.map(|c| match c {
            Coder::Hardware => Decoding::Hardware,
            Coder::Software => Decoding::Software,
        });
        args.bitrate = self.bitrate;
    }
}

impl Filters {
    pub fn filter(self) -> Filter {
        Filter {
            clip: self.clip,
            track: self.track,
            index: self.index,
            at: self.at,
            marker: self.marker,
            meta: self.meta,
        }
    }
}

/// The arguments of `motion check`, `motion render`, `timeline check` and `timeline render`.
pub fn scene_args(
    file: String,
    timeline: bool,
    bind: Option<Bindings>,
    frames: Option<FrameChoice>,
    render: Option<Output>,
) -> Result<Args, Failure> {
    let mut args = Args {
        file,
        timeline,
        ..Args::default()
    };
    if let Some(bind) = bind {
        bind.apply(&mut args)?;
    }
    if let Some(frames) = frames {
        args.frames = frames.get();
    }
    if let Some(render) = render {
        args.out = Some(render.out);
        render.options.apply(&mut args);
    }
    Ok(args)
}

/// The arguments of `project render`, for its timeline.
pub fn project_render_args(file: String, out: Option<String>, options: RenderOptions) -> Args {
    let mut args = Args {
        file,
        timeline: true,
        out,
        ..Args::default()
    };
    options.apply(&mut args);
    args
}

fn pair(text: &str) -> Result<(String, String), String> {
    match text.split_once('=') {
        Some((name, path)) if !name.is_empty() && !path.is_empty() => {
            Ok((name.to_string(), path.to_string()))
        }
        _ => Err("it is NAME=PATH".into()),
    }
}

fn meta(text: &str) -> Result<(String, String), String> {
    match text.split_once('=') {
        Some((key, value)) if !key.is_empty() => Ok((key.to_string(), value.to_string())),
        _ => Err("it is KEY=VALUE".into()),
    }
}

fn index(text: &str) -> Result<usize, String> {
    text.parse()
        .map_err(|_| "it is a JSON array position from 0".into())
}

fn frame(text: &str) -> Result<i64, String> {
    (text.parse().ok().filter(|n| *n >= 0)).ok_or_else(|| "a frame is a number from 0".into())
}

fn frames(text: &str) -> Result<Frames, String> {
    if let Some((a, b)) = text.split_once("..") {
        let (a, b) = (frame(a)?, frame(b)?);
        if b < a {
            return Err(format!("{a}..{b} ends before it starts"));
        }
        return Ok(Frames::Range(a, b));
    }
    let list = text.split(',').map(frame).collect::<Result<Vec<_>, _>>()?;
    Ok(Frames::List(list))
}

fn size(text: &str) -> Result<[u32; 2], String> {
    (text.split_once('x'))
        .and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))
        .ok_or_else(|| "it is WIDTHxHEIGHT, such as 3840x2160".into())
}

fn bits(text: &str) -> Result<u64, String> {
    let (number, scale) = match text.as_bytes().last() {
        Some(b'k' | b'K') => (&text[..text.len() - 1], 1e3),
        Some(b'm' | b'M') => (&text[..text.len() - 1], 1e6),
        _ => (text, 1.0),
    };
    let n: f64 = number
        .parse()
        .map_err(|_| "it is bits a second, such as 8M")?;
    match n > 0.0 && n.is_finite() {
        true => Ok((n * scale) as u64),
        false => Err("it is bits a second above 0, such as 8M".into()),
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    fn parse(words: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("valle").chain(words.iter().copied()))
    }

    fn scene(words: &[&str]) -> Args {
        match parse(words)
            .unwrap_or_else(|e| panic!("{words:?}: {e}"))
            .command
        {
            Command::Motion(Motion::Render { file, bind, render }) => {
                scene_args(file, false, Some(bind), None, Some(render))
                    .unwrap_or_else(|e| panic!("{}", e.message))
            }
            Command::Motion(Motion::Check { file, bind, frames }) => {
                scene_args(file, false, Some(bind), Some(frames), None)
                    .unwrap_or_else(|e| panic!("{}", e.message))
            }
            _ => panic!("not a motion command"),
        }
    }

    fn refused(words: &[&str], why: &str) {
        let e = parse(words)
            .err()
            .unwrap_or_else(|| panic!("accepted {words:?}"));
        assert!(e.to_string().contains(why), "{words:?}: {e}");
    }

    #[test]
    fn commands_and_options_are_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn options_belong_to_their_commands() {
        refused(&["motion", "render", "a.motion.tsx"], "--out");
        refused(&["motion", "check", "a.motion.tsx", "-o", "x.png"], "'-o'");
        refused(
            &["timeline", "check", "t.timeline.json", "--props", "{}"],
            "'--props'",
        );
        refused(
            &["timeline", "query", "t.timeline.json", "--codec", "h264"],
            "'--codec'",
        );
        refused(
            &["motion", "check", "a.motion.tsx", "b.motion.tsx"],
            "'b.motion.tsx'",
        );
        refused(
            &[
                "motion",
                "render",
                "a.motion.tsx",
                "-o",
                "x.mp4",
                "--codec",
                "av1",
            ],
            "av1",
        );
        refused(
            &[
                "motion",
                "render",
                "a.motion.tsx",
                "-o",
                "x.png",
                "--frame",
                "1",
                "--frames",
                "2",
            ],
            "cannot be used with",
        );
        refused(&["project", "restore", "0"], "0");
        refused(&["project", "add"], "PATH");
    }

    #[test]
    fn frame_lists_keep_order_and_ranges_include_both_ends() {
        let a = scene(&[
            "motion",
            "check",
            "a.motion.tsx",
            "--json",
            "--frames",
            "3,0,3",
            "--events",
        ]);
        assert!(matches!(a.frames, Some(Frames::List(v)) if v == [3, 0, 3]));
        let a = scene(&["motion", "check", "a.motion.tsx", "--frames", "0..0"]);
        assert!(matches!(a.frames, Some(Frames::Range(0, 0))));
        let a = scene(&["motion", "check", "a.motion.tsx", "--frame", "7"]);
        assert!(matches!(a.frames, Some(Frames::List(v)) if v == [7]));
        for s in ["", "1.5", "1,", "0..", "1..2..3", "9223372036854775808"] {
            refused(
                &["motion", "check", "a.motion.tsx", "--frames", s],
                "a frame is a number",
            );
        }
        refused(
            &["motion", "check", "a.motion.tsx", "--frames", "-1"],
            "a frame is a number",
        );
        refused(
            &["motion", "check", "a.motion.tsx", "--frame", "-1"],
            "a frame is a number",
        );
        refused(
            &["motion", "check", "a.motion.tsx", "--frames", "5..2"],
            "ends before",
        );
    }

    #[test]
    fn bindings_keep_resource_lists_and_separate_props_from_data() {
        let a = scene(&[
            "motion",
            "check",
            "a.motion.tsx",
            "--props",
            r#"{"title":"Hi"}"#,
            "--data",
            r#"{"values":[1,2]}"#,
            "--resource",
            "photo=a.png",
            "--resource",
            "photo=b=x.png",
            "--input",
            "plate=c.png",
            "--caption",
            r#"{"runs":[{"text":"Hello"}]}"#,
        ]);
        assert_eq!(a.inputs.resources["photo"], ["a.png", "b=x.png"]);
        assert_eq!(a.inputs.pictures["plate"], "c.png");
        assert_eq!(a.inputs.props["title"], "Hi");
        assert_eq!(a.inputs.data["values"], serde_json::json!([1, 2]));
        assert_eq!(a.inputs.caption.unwrap()["runs"][0]["text"], "Hello");
        for flag in ["--props", "--data"] {
            let Command::Motion(Motion::Check { file, bind, frames }) =
                parse(&["motion", "check", "a.motion.tsx", flag, "[]"])
                    .unwrap()
                    .command
            else {
                unreachable!()
            };
            let Err(e) = scene_args(file, false, Some(bind), Some(frames), None) else {
                panic!("{flag} [] accepted")
            };
            assert!(e.message.contains("JSON object"), "{}", e.message);
        }
        for flag in ["--resource", "--input"] {
            for s in ["photo", "=a.png", "photo="] {
                refused(&["motion", "check", "a.motion.tsx", flag, s], "NAME=PATH");
            }
        }
    }

    #[test]
    fn render_options_are_typed_and_recorded_by_name() {
        let a = scene(&[
            "motion",
            "render",
            "a.motion.tsx",
            "-o",
            "x.mp4",
            "--codec",
            "hevc",
            "--encoder",
            "hardware",
            "--decoder",
            "ffmpeg",
            "--bitrate",
            "8M",
            "--output-size",
            "3840x2160",
            "--fps",
            "60000/1001",
        ]);
        assert_eq!(a.out.as_deref(), Some("x.mp4"));
        assert_eq!(a.codec, Some(Codec::Hevc));
        assert_eq!(a.encoder, Some(Encoding::Hardware));
        assert_eq!(a.decoder, Some(Decoding::Software));
        assert_eq!(a.bitrate, Some(8_000_000));
        let names: Vec<_> = a
            .options()
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        assert_eq!(
            names,
            [
                "output-size=3840x2160",
                "fps=60000/1001",
                "codec=hevc",
                "encoder=vt",
                "decoder=ffmpeg",
                "bitrate=8000000"
            ]
        );
        for size in ["0x100", "9000x9000"] {
            let a = scene(&[
                "motion",
                "render",
                "a.motion.tsx",
                "-o",
                "x.png",
                "--output-size",
                size,
            ]);
            assert!(a.output_size([1920, 1080]).is_err(), "{size}");
        }
        for size in ["-1x100", "3840", "3840x2160x2"] {
            refused(
                &[
                    "motion",
                    "render",
                    "a.motion.tsx",
                    "-o",
                    "x.png",
                    "--output-size",
                    size,
                ],
                "WIDTHxHEIGHT",
            );
        }
        for bits in ["0", "-1", "fast", "M", "inf"] {
            refused(
                &[
                    "motion",
                    "render",
                    "a.motion.tsx",
                    "-o",
                    "x.mp4",
                    "--bitrate",
                    bits,
                ],
                "bits a second",
            );
        }
    }

    #[test]
    fn global_options_and_project_folder_go_anywhere() {
        let cli = parse(&["project", "save", "-m", "first", "--json", "-C", "/tmp/p"]).unwrap();
        assert!(cli.json);
        let Command::Project(p) = cli.command else {
            panic!("not a project command")
        };
        assert_eq!(p.folder.as_deref(), Some(std::path::Path::new("/tmp/p")));
        assert!(matches!(p.command, ProjectCommand::Save { message: Some(m) } if m == "first"));
    }

    /// Words as a shell splits a command line written in a guide: by spaces, keeping what
    /// single or double quotes hold together.
    fn shell_words(line: &str) -> Vec<String> {
        let (mut words, mut word, mut quote) = (Vec::new(), String::new(), None);
        for c in line.chars() {
            match c {
                '\'' | '"' if quote.is_none() => quote = Some(c),
                c if quote == Some(c) => quote = None,
                ' ' if quote.is_none() => {
                    if !word.is_empty() {
                        words.push(std::mem::take(&mut word));
                    }
                }
                c => word.push(c),
            }
        }
        if !word.is_empty() {
            words.push(word);
        }
        words
    }

    #[test]
    fn every_command_the_guides_give_parses() {
        let mut seen = 0;
        for guide in &crate::docs::GUIDES {
            let mut code = false;
            for line in guide.content.lines() {
                if line.starts_with("```") {
                    code = !code && line.trim_start_matches('`').starts_with("sh");
                    continue;
                }
                let line = line.split(" # ").next().unwrap_or(line).trim();
                let Some(rest) = line.strip_prefix("valle ").filter(|_| code) else {
                    continue;
                };
                if rest.contains("<command>") {
                    continue;
                }
                let words = shell_words(rest);
                let words: Vec<&str> = words.iter().map(String::as_str).collect();
                match parse(&words) {
                    Ok(_) => {}
                    // Help is a parse that prints instead of running.
                    Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => {}
                    Err(e) => panic!("{}: valle {rest}: {e}", guide.topic),
                }
                seen += 1;
            }
        }
        assert!(seen >= 30, "the guides give {seen} commands");
    }

    #[test]
    fn storyboard_options_require_storyboard_and_one_positive_sampling_mode() {
        let base = ["motion", "render", "a.motion.tsx", "-o", "sheet.png"];
        let a = scene(
            &[
                &base[..],
                &["--storyboard", "--samples", "60", "--per-page", "12"],
            ]
            .concat(),
        );
        assert_eq!(a.samples, Some(60));
        assert_eq!(a.per_page, Some(12));
        assert_eq!(a.step, None);
        for option in ["--samples", "--step", "--per-page"] {
            refused(&[&base[..], &[option, "2"]].concat(), "--storyboard");
            for invalid in ["0", "-1", "1.5"] {
                assert!(parse(&[&base[..], &["--storyboard", option, invalid]].concat()).is_err());
            }
        }
        for (left, right) in [
            ("--samples", "--step"),
            ("--samples", "--frames"),
            ("--step", "--frames"),
            ("--samples", "--frame"),
            ("--step", "--frame"),
        ] {
            refused(
                &[&base[..], &["--storyboard", left, "2", right, "2"]].concat(),
                "cannot be used with",
            );
        }
    }

    #[test]
    fn render_frame_rates_are_positive_and_exact() {
        let base = ["motion", "render", "a.motion.tsx", "-o", "x.png", "--fps"];
        for (written, ratio) in [
            ("60", (60, 1)),
            ("29.97", (2997, 100)),
            ("30000/1001", (30000, 1001)),
        ] {
            let a = scene(&[&base[..], &[written]].concat());
            let fps = a.fps.unwrap();
            assert_eq!((fps.num(), fps.den()), ratio);
        }
        for bad in ["0", "-30", "30/0", "0/1001", "NaN", "inf", "30/1/2"] {
            assert!(parse(&[&base[..], &[bad]].concat()).is_err(), "{bad}");
        }
    }
}
