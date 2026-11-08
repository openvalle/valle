//! A scene's role in a Timeline: what `export const role = …` declares, read without running the
//! scene. Without one it is a plain clip.

use oxc::ast::ast::{Expression, ObjectExpression};
use serde_json::{Map, Value};

use super::controls;
use super::rewrite::bare;
use crate::time::Tick;

/// How a scene is used in a Timeline.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Role {
    /// A clip, played as it is, trimmed or sped up.
    #[default]
    Clip,
    /// A title or a lower third over the clips: its intro and outro keep their length when the
    /// clip is longer or shorter than the scene, and its middle holds as `hold` says.
    Overlay {
        intro: Tick,
        outro: Tick,
        hold: Hold,
    },
    /// Goes from one clip to the next, reading them as `input://from` and `input://to`.
    Transition,
    /// Works on a clip's picture, or on all under an adjustment clip, as `input://source`.
    Effect,
    /// Shows a caption track's captions, each as its `caption` prop, with an intro and an outro
    /// as an overlay has them, its middle held once.
    CaptionPresenter { intro: Tick, outro: Tick },
}

/// What an overlay's middle does in a clip of another length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hold {
    /// Plays once, then holds its last frame until the outro.
    Once,
    /// Plays again and again.
    Loop,
    /// Plays slower or faster to fill the clip's middle.
    Stretch,
}

const ROLES: &str = "overlay({ intro, outro, hold }), transition(), effect() or \
                     captionPresenter({ intro, outro })";

/// The role `e` declares: `overlay({ intro: 0.5, outro: 0.5, hold: "loop" })` and the like.
pub fn read(e: &Expression<'_>) -> Result<Role, String> {
    let written =
        || format!("role must be one of {ROLES}: tools read it without running the scene");
    let Expression::CallExpression(call) = bare(e) else {
        return Err(written());
    };
    let Expression::Identifier(f) = &call.callee else {
        return Err(written());
    };
    let name = f.name.as_str();
    let mut options = match call.arguments.as_slice() {
        [] => Map::new(),
        [a] => match a.as_expression().map(bare) {
            Some(Expression::ObjectExpression(o)) => options(o, name)?,
            _ => return Err(format!("role {name}() takes an object of its options")),
        },
        _ => return Err(format!("role {name}() takes an object of its options")),
    };
    let mut seconds = |k: &str| match options.remove(k) {
        None => Ok(Tick::ZERO),
        Some(Value::Number(n)) if n.as_f64().is_some_and(|x| x >= 0.0) => {
            Tick::from_seconds(n.as_f64().unwrap_or_default())
        }
        Some(v) => Err(format!("role {name}() {k} is seconds, 0 or more, not {v}")),
    };
    let role = match name {
        "overlay" => {
            let (intro, outro) = (seconds("intro")?, seconds("outro")?);
            let hold = match options.remove("hold") {
                None => Hold::Once,
                Some(Value::String(s)) if s == "once" => Hold::Once,
                Some(Value::String(s)) if s == "loop" => Hold::Loop,
                Some(Value::String(s)) if s == "stretch" => Hold::Stretch,
                Some(v) => {
                    return Err(format!(
                        "role overlay() hold is \"once\", \"loop\" or \"stretch\", not {v}"
                    ));
                }
            };
            Role::Overlay { intro, outro, hold }
        }
        "captionPresenter" => Role::CaptionPresenter {
            intro: seconds("intro")?,
            outro: seconds("outro")?,
        },
        "transition" => Role::Transition,
        "effect" => Role::Effect,
        _ => return Err(written()),
    };
    if let Some(k) = options.keys().next() {
        let takes = match role {
            Role::Overlay { .. } => "it takes intro, outro and hold",
            Role::CaptionPresenter { .. } => "it takes intro and outro",
            _ => "it takes none",
        };
        return Err(format!("role {name}() has no option {k}; {takes}"));
    }
    Ok(role)
}

fn options(o: &ObjectExpression<'_>, name: &str) -> Result<Map<String, Value>, String> {
    controls::object(o).map_err(|e| format!("role {name}(): {e}"))
}

impl Role {
    /// The pictures the Timeline gives a scene of the role, which it reads as `input://name`.
    pub fn inputs(self) -> &'static [&'static str] {
        match self {
            Role::Effect => &["source"],
            Role::Transition => &["from", "to"],
            _ => &[],
        }
    }

    /// The intro and outro the scene keeps in a clip of any length, if it has them.
    pub fn ends(self) -> Option<(Tick, Tick)> {
        match self {
            Role::Overlay { intro, outro, .. } | Role::CaptionPresenter { intro, outro } => {
                Some((intro, outro))
            }
            _ => None,
        }
    }

    /// The scene's time at `t` into a clip `clip` long, for a scene `duration` long: its intro at
    /// the clip's start, its outro at its end, and its middle between as its hold says; the
    /// clip's time itself for a role without an intro and an outro.
    pub fn time(self, duration: Tick, clip: Tick, t: Tick) -> Result<Tick, String> {
        let Some((intro, outro)) = self.ends() else {
            return Ok(t);
        };
        if clip < intro + outro {
            return Err(format!(
                "the clip is {} s, shorter than the intro and the outro, {} s",
                clip.seconds_text(),
                (intro + outro).seconds_text()
            ));
        }
        let hold = match self {
            Role::Overlay { hold, .. } => hold,
            _ => Hold::Once,
        };
        // The middles: the scene's and the clip's.
        let middle = (duration - intro - outro).0;
        let room = (clip - intro - outro).0;
        Ok(if t < intro {
            t
        } else if t >= clip - outro {
            duration - (clip - t)
        } else {
            let into = (t - intro).0;
            intro
                + Tick(match hold {
                    Hold::Once => into.min(middle),
                    Hold::Loop if middle > 0 => into % middle,
                    Hold::Stretch if room > 0 => {
                        (into as i128 * middle as i128 / room as i128) as i64
                    }
                    Hold::Loop | Hold::Stretch => 0,
                })
        })
    }
}

/// The words of the caption a presenter shows alone.
const PLACEHOLDER: &str = "Each word lights up as it is said";

/// The `caption` prop a presenter takes: `bound`, `{ text, runs }`, checked, each run a word
/// with its start and end in seconds into the clip, in order; or alone, a placeholder whose
/// words come one by one between the intro and the outro of a scene `duration` long.
pub fn caption(bound: Option<&Value>, role: Role, duration: Tick) -> Result<Value, String> {
    let Some(bound) = bound else {
        let (intro, outro) = role.ends().unwrap_or_default();
        let words: Vec<&str> = PLACEHOLDER.split(' ').collect();
        let step = (duration - intro - outro).seconds() / words.len() as f64;
        let runs: Vec<Value> = (words.iter().enumerate())
            .map(|(i, w)| {
                let start = intro.seconds() + step * i as f64;
                serde_json::json!({ "text": w, "start": start, "end": start + step })
            })
            .collect();
        return Ok(serde_json::json!({ "text": PLACEHOLDER, "runs": runs }));
    };
    let shape =
        "is { text, runs }, each run { text, start, end, style? } with its seconds into the clip";
    let Value::Object(o) = bound else {
        return Err(format!("caption {shape}"));
    };
    if o.len() != 2 || !matches!(o.get("text"), Some(Value::String(_))) {
        return Err(format!("caption {shape}"));
    }
    let Some(Value::Array(runs)) = o.get("runs") else {
        return Err(format!("caption {shape}"));
    };
    let mut last = 0.0;
    for (i, run) in runs.iter().enumerate() {
        let number = |k: &str| run.get(k).and_then(Value::as_f64);
        let (Some(start), Some(end)) = (number("start"), number("end")) else {
            return Err(format!("caption runs[{i}] {shape}"));
        };
        let words = run.get("text").is_some_and(Value::is_string);
        if !words
            || run.as_object().is_none_or(|r| {
                r.keys()
                    .any(|k| !matches!(k.as_str(), "text" | "start" | "end" | "style"))
            })
        {
            return Err(format!("caption runs[{i}] {shape}"));
        }
        if run.get("style").is_some_and(|s| !s.is_object()) {
            return Err(format!(
                "caption runs[{i}].style is an object of style overrides"
            ));
        }
        if start < last || end < start {
            return Err(format!(
                "caption runs[{i}] is from {start} to {end} s: runs come in order, each ending \
                 after it starts"
            ));
        }
        last = start;
    }
    Ok(bound.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: f64) -> Tick {
        Tick::from_seconds(x).unwrap()
    }

    #[test]
    fn overlays_keep_their_ends_and_hold_their_middle() {
        let at = |hold, clip: f64, t: f64| {
            let role = Role::Overlay {
                intro: s(1.0),
                outro: s(1.0),
                hold,
            };
            role.time(s(4.0), s(clip), s(t)).unwrap().seconds()
        };
        // Intro and outro as they are; the scene's middle is 2 s, the clip's 6 s.
        for hold in [Hold::Once, Hold::Loop, Hold::Stretch] {
            assert_eq!(at(hold, 8.0, 0.5), 0.5);
            assert_eq!(at(hold, 8.0, 7.5), 3.5);
            assert_eq!(at(hold, 8.0, 7.0), 3.0);
        }
        assert_eq!(at(Hold::Once, 8.0, 2.5), 2.5);
        assert_eq!(at(Hold::Once, 8.0, 5.0), 3.0);
        assert_eq!(at(Hold::Loop, 8.0, 3.5), 1.5);
        assert_eq!(at(Hold::Stretch, 8.0, 4.0), 2.0);
        // A shorter clip cuts its middle short, or plays it faster.
        assert_eq!(at(Hold::Once, 3.0, 1.5), 1.5);
        assert_eq!(at(Hold::Once, 3.0, 2.5), 3.5);
        assert_eq!(at(Hold::Stretch, 3.0, 1.5), 2.0);
        // No middle at all.
        assert_eq!(at(Hold::Loop, 2.0, 1.5), 3.5);
        let short = Role::Overlay {
            intro: s(1.0),
            outro: s(1.0),
            hold: Hold::Once,
        };
        assert_eq!(
            short.time(s(4.0), s(1.5), s(0.0)).unwrap_err(),
            "the clip is 1.5 s, shorter than the intro and the outro, 2 s"
        );
        assert_eq!(Role::Effect.time(s(4.0), s(9.0), s(6.0)), Ok(s(6.0)));
    }

    #[test]
    fn presenters_take_captions_checked_or_a_placeholder() {
        let role = Role::CaptionPresenter {
            intro: s(1.0),
            outro: s(1.0),
        };
        let shown = caption(None, role, s(10.0)).unwrap();
        let runs = shown["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 8);
        assert_eq!(
            (runs[0]["text"].as_str(), runs[0]["start"].as_f64()),
            (Some("Each"), Some(1.0))
        );
        assert_eq!(runs[7]["end"].as_f64(), Some(9.0));
        let bound = serde_json::json!({ "text": "Hi there", "runs": [
            { "text": "Hi", "start": 0.5, "end": 1 },
            { "text": "there", "start": 1, "end": 1.5 },
        ] });
        assert_eq!(caption(Some(&bound), role, s(10.0)), Ok(bound));
        let styled = serde_json::json!({ "text": "Hello", "runs": [
            { "text": "Hello", "start": 0, "end": 1, "style": { "textColor": "red" } }
        ] });
        assert_eq!(caption(Some(&styled), role, s(10.0)), Ok(styled.clone()));
        let mut invalid = styled;
        invalid["runs"][0]["style"] = serde_json::json!([]);
        assert!(
            caption(Some(&invalid), role, s(10.0))
                .unwrap_err()
                .contains("runs[0].style")
        );
        let late = serde_json::json!({ "text": "a b", "runs": [
            { "text": "a", "start": 2, "end": 3 },
            { "text": "b", "start": 1, "end": 2 },
        ] });
        assert_eq!(
            caption(Some(&late), role, s(10.0)).unwrap_err(),
            "caption runs[1] is from 1 to 2 s: runs come in order, each ending after it starts"
        );
        let bare = serde_json::json!("Hi");
        assert!(
            caption(Some(&bare), role, s(10.0))
                .unwrap_err()
                .starts_with("caption is {")
        );
    }
}
