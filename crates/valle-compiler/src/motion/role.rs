use super::*;
use valle_timeline::wire::timeline::TimelineTimeWire;
use valle_timeline::{MotionRole, OverlayHold, RationalTime};

impl Compiler<'_> {
    pub(super) fn compile_role(&mut self, expression: &Expression<'_>) {
        let Some(value) = self.eval_static(expression) else {
            return;
        };
        let parse = || -> Result<MotionRole, String> {
            let obj = value
                .as_object()
                .ok_or("role must be overlay({intro:seconds(...),outro:seconds(...),hold:...})")?;
            if obj.get("__valleType").and_then(|v| v.as_str()) != Some("overlay") {
                return Err("role must use overlay(...)".into());
            }
            if let Some(key) = obj
                .keys()
                .find(|key| !["__valleType", "intro", "outro", "hold"].contains(&key.as_str()))
            {
                return Err(format!("overlay has no field `{key}`"));
            }
            let time = |name: &str| -> Result<RationalTime, String> {
                let v = obj
                    .get(name)
                    .ok_or_else(|| format!("overlay needs `{name}: seconds(...)`"))?;
                if v.get("__valleType").and_then(|v| v.as_str()) != Some("sequenceTime")
                    || v.get("unit").and_then(|v| v.as_str()) != Some("seconds")
                    || v.as_object().is_none_or(|v| v.len() != 3)
                {
                    return Err(format!("overlay {name} must use seconds(...)"));
                }
                let value = v
                    .get("value")
                    .and_then(|v| v.as_number())
                    .ok_or_else(|| format!("overlay {name} needs finite seconds"))?;
                let t = TimelineTimeWire::new(value.to_string()).map_err(|e| e.to_string())?;
                Ok(RationalTime::from_exact(t.to_exact()))
            };
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
