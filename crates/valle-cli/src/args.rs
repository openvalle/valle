//! A command's arguments: its file, its output, and the values bound to the scene.

use serde_json::{Map, Value};
use valle_engine::front::Inputs;
use valle_native::media::{Codec, Decoding, Encoding};

use crate::Failure;

/// What `crate::cli` read for a command on a scene or a timeline.
#[derive(Default)]
pub struct Args {
    /// The scene, or the document, the command works on.
    pub file: String,
    /// Whether `file` is a timeline, not a scene.
    pub timeline: bool,
    pub out: Option<String>,
    pub inputs: Inputs,
    /// The frames to draw, if not all.
    pub frames: Option<Frames>,
    pub storyboard: bool,
    /// Output pixels, if not the canvas's.
    pub output_size: Option<[u32; 2]>,
    pub codec: Option<Codec>,
    pub encoder: Option<Encoding>,
    pub decoder: Option<Decoding>,
    /// A video's bits a second.
    pub bitrate: Option<u64>,
}

#[derive(Clone)]
pub enum Frames {
    List(Vec<i64>),
    /// From the first to the last, both drawn.
    Range(i64, i64),
}

impl Args {
    /// The render options given, by their option's name: what a render record keeps.
    pub fn options(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        if let Some([w, h]) = self.output_size {
            out.push(("output-size", format!("{w}x{h}")));
        }
        if let Some(codec) = self.codec {
            let name = match codec {
                Codec::H264 => "h264",
                Codec::Hevc => "hevc",
                Codec::ProRes4444 => "prores",
            };
            out.push(("codec", name.into()));
        }
        let coder = |hardware: bool| if hardware { "vt" } else { "ffmpeg" }.to_string();
        if let Some(e) = self.encoder {
            out.push(("encoder", coder(e == Encoding::Hardware)));
        }
        if let Some(d) = self.decoder {
            out.push(("decoder", coder(d == Decoding::Hardware)));
        }
        if let Some(bits) = self.bitrate {
            out.push(("bitrate", bits.to_string()));
        }
        out
    }

    /// Output pixels, leaving the scene's logical canvas unchanged.
    pub fn output_size(&self, canvas: [u32; 2]) -> Result<[u32; 2], Failure> {
        let size = self.output_size.unwrap_or(canvas);
        valle_engine::gpu::output_size(size).map_err(|e| Failure::new("usage", e))?;
        Ok(size)
    }
}

/// JSON written out, or `@path` for a file of it.
pub fn json(text: &str, what: &str) -> Result<Value, Failure> {
    let text = match text.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| Failure::new("not_found", format!("{what} {path}: {e}")))?,
        None => text.to_string(),
    };
    serde_json::from_str(&text).map_err(|e| Failure::new("usage", format!("{what}: {e}")))
}

pub fn object(text: &str, what: &str) -> Result<Map<String, Value>, Failure> {
    match json(text, what)? {
        Value::Object(o) => Ok(o),
        _ => Err(Failure::new(
            "usage",
            format!("{what} is a JSON object of values by name"),
        )),
    }
}
