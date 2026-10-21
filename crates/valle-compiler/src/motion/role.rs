use super::*;
use valle_timeline::wire::timeline::TimelineTimeWire;
use valle_timeline::{MotionRole, OverlayHold, RationalTime};

impl Compiler<'_> {
    pub(super) fn compile_role(&mut self, expression: &Expression<'_>) {
        const ROLES: &str =
            "role must use overlay(...), captionPresenter(...), transition() or effect()";
        let reported = self.diagnostics.len();
        let Some(value) = self.eval_static(expression) else {
            // A value that is no JSON, such as an uncalled `transition`, would otherwise leave
            // the component an ordinary clip without a word.
            if self.diagnostics.len() == reported {
                self.illegal(DiagCode::ModuleShape, expression.span(), ROLES);
            }
            return;
        };
        let parse = || -> Result<MotionRole, String> {
            let obj = value.as_object().ok_or(ROLES)?;
            let kind = obj.get("__valleType").and_then(|v| v.as_str());
            let allowed: &[&str] = match kind {
                Some("overlay") => &["__valleType", "intro", "outro", "hold"],
                Some("captionPresenter") => &["__valleType", "intro", "outro"],
                Some("transition" | "effect") => &["__valleType"],
                _ => return Err(ROLES.into()),
            };
            let kind = kind.unwrap();
            if let Some(key) = obj.keys().find(|key| !allowed.contains(&key.as_str())) {
                return Err(format!("{kind} has no field `{key}`"));
            }
            // A transition or an effect always spans its window; it has no timing of its own.
            match kind {
                "transition" => return Ok(MotionRole::Transition),
                "effect" => return Ok(MotionRole::Effect),
                _ => {}
            }
            let time = |name: &str| -> Result<RationalTime, String> {
                let v = obj
                    .get(name)
                    .ok_or_else(|| format!("{kind} needs `{name}: seconds(...)`"))?;
                if v.get("__valleType").and_then(|v| v.as_str()) != Some("sequenceTime")
                    || v.get("unit").and_then(|v| v.as_str()) != Some("seconds")
                    || v.as_object().is_none_or(|v| v.len() != 3)
                {
                    return Err(format!("{kind} {name} must use seconds(...)"));
                }
                let value = v
                    .get("value")
                    .and_then(|v| v.as_number())
                    .ok_or_else(|| format!("{kind} {name} needs finite seconds"))?;
                let t = TimelineTimeWire::new(value.to_string()).map_err(|e| e.to_string())?;
                Ok(RationalTime::from_exact(t.to_exact()))
            };
            if kind == "captionPresenter" {
                return Ok(MotionRole::CaptionPresenter {
                    intro: time("intro")?,
                    outro: time("outro")?,
                });
            }
            let hold = match obj.get("hold").and_then(|v| v.as_str()) {
                Some("once") => OverlayHold::Once,
                Some("loop") => OverlayHold::Loop,
                Some("stretch") => OverlayHold::Stretch,
                _ => return Err("overlay hold must be once, loop or stretch".into()),
            };
            Ok(MotionRole::Overlay {
                intro: time("intro")?,
                outro: time("outro")?,
                hold,
            })
        };
        match parse() {
            Ok(role) => self.role = role,
            Err(message) => self.illegal(DiagCode::ModuleShape, expression.span(), message),
        }
    }
}
