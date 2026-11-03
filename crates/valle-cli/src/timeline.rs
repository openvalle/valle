//! `valle timeline check` and `valle timeline render`: a document lowered to the scene it plays
//! as, which the Motion commands then check and draw.

use std::path::Path;

use valle_engine::time::Tick;
use valle_engine::timeline::{self, Host, Lowered};
use valle_native::media;

use crate::Failure;
use crate::args::Args;

pub fn lower(args: &Args) -> Result<Lowered, Failure> {
    let i = &args.inputs;
    if !(i.props.is_empty() && i.data.is_empty() && i.resources.is_empty())
        || !i.pictures.is_empty()
        || i.caption.is_some()
    {
        return Err(Failure::new(
            "usage",
            "a timeline binds its components' props, data and resources itself",
        )
        .hint("--props, --data, --resource, --input and --caption are for valle motion"));
    }
    let path = args.file.replace('\\', "/");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    let doc = valle_timeline::parse(&text)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))?;
    let read = |p: &str| std::fs::read_to_string(p).ok();
    let length = |p: &str| media::length(Path::new(p)).ok().map(Tick);
    let host = Host {
        read: &read,
        length: &length,
    };
    timeline::lower(&path, &doc, &host)
        .map_err(|e| Failure::new("timeline", format!("{path}: {e}")))
}
