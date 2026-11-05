//! Validation of a scene across time, without constructing a renderable frame. This shares
//! expression, style, resource and layout checks with rendering; it does not memoize JS calls.

use super::*;

/// Checks a loaded scene. Consuming the scene keeps resource bindings fixed for the lifetime
/// of its validation results. Rendering uses its own scene, not cached check results.
pub struct Checker {
    scene: Scene,
    plan: Option<Plan>,
}

/// Only the current variant is retained here; changing variants drops its successful values.
struct Plan {
    template: Rc<Template>,
    layout: bool,
    late: bool,
    inputs: Vec<u32>,
    checked: Option<Vec<V>>,
}

impl Plan {
    fn new(template: Rc<Template>) -> Self {
        let late = template
            .program
            .ops
            .iter()
            .any(|op| matches!(op, Op::Bounds(..)));
        // Layout's fallible work is text measurement, shaping and decoration. Boxes alone
        // need no layout for validation. A bounds() value does need their actual positions.
        let layout = late
            || template
                .nodes
                .iter()
                .any(|n| matches!(n.kind, Kind::Text { .. }));
        // Guard inputs have already been checked by Front. Only values consumed by nodes
        // can invalidate the remaining checks. Keep direct outputs, not all their operands:
        // changing seconds need not invalidate a width that still evaluates to 20px.
        let mut inputs = Vec::new();
        let mut prop = |p: &Prop| {
            if let Prop::Expr(e) = p {
                inputs.push(*e);
            }
        };
        for n in &template.nodes {
            for (_, p) in &n.style {
                prop(p);
            }
            match &n.kind {
                Kind::View | Kind::Mask { .. } => {}
                Kind::Text { content } | Kind::Span { content, .. } => {
                    for p in content {
                        if let Piece::Text(p) = p {
                            prop(p);
                        }
                    }
                }
                Kind::Image { video, .. } => {
                    if let Some(p) = video {
                        prop(p);
                    }
                }
                Kind::Svg { attributes, .. } => {
                    for (_, p) in attributes {
                        prop(p);
                    }
                }
                Kind::Shader { uniforms, .. } => {
                    for (_, parts) in uniforms {
                        for p in parts {
                            prop(p);
                        }
                    }
                }
            }
        }
        inputs.sort_unstable();
        inputs.dedup();
        Self {
            template,
            layout,
            late,
            inputs,
            checked: None,
        }
    }
}

impl Checker {
    pub(super) fn new(scene: Scene) -> Self {
        Self { scene, plan: None }
    }

    /// The same tracing diagnostics as rendering, for checks of all or selected frames.
    pub fn traces(&self) -> (usize, Option<&str>) {
        self.scene.traces()
    }

    /// Validate the frame at `at`, including sampled-time and video decode errors. No drawing
    /// commands, box pictures, final transforms or motion-blur velocities are produced.
    pub fn check(&mut self, at: Tick) -> Result<(), Error> {
        // Evaluate all operations, even unused fallible expressions. A successful first frame
        // is not evidence that a changing value or a later branch is valid.
        let frame = self.scene.front.frame(at)?;
        let template = frame.template;
        if self
            .plan
            .as_ref()
            .is_none_or(|p| !Rc::ptr_eq(&p.template, &template))
        {
            self.plan = Some(Plan::new(template.clone()));
        }
        let plan = self.plan.as_mut().expect("the current variant has a plan");
        if plan.checked.as_ref().is_some_and(|previous| {
            plan.inputs
                .iter()
                .zip(previous)
                .all(|(&e, v)| frame.values[e as usize] == *v)
        }) {
            return Ok(());
        }
        plan.checked = None;
        let needs_layout = plan.layout;
        let late = plan.late;
        let styles = self.scene.styled(&template, &frame.values)?;
        let sampled = styles.iter().any(|s| s.motion_auto);
        let (values, styles, layout, natural) = if needs_layout || sampled {
            let natural = self.scene.pictures.iter().map(Picture::natural).collect();
            let laid = self
                .scene
                .layout(at, template.clone(), frame.values, styles)?;
            if sampled {
                self.trails(at, &template, &laid.values, &laid.styles)?;
            }
            (laid.values, laid.styles, Some(laid.layout), natural)
        } else {
            // Do not leave a previous variant's layout alive when this one needs none.
            self.scene.retained = None;
            (frame.values, styles, None, Vec::new())
        };
        self.scene.text.sweep();
        self.scene.show_videos(&template, &values, &styles)?;
        backgrounds(&template, &styles, &self.scene.pictures)?;
        if let Some(layout) = layout {
            self.scene.retained = Some(RetainedLayout {
                text: text_values(&template, &values).cloned().collect(),
                template,
                styles: Rc::new(styles),
                natural,
                layout,
            });
        }
        // Equal values of the same variant have the same checks, with fixed resource bindings.
        // Subframe inputs can change while centre-frame values do not; never reuse those checks.
        // Late values have been settled above and are not comparable to the pre-layout values.
        if !sampled && !late {
            let plan = self.plan.as_mut().unwrap();
            plan.checked = Some(
                plan.inputs
                    .iter()
                    .map(|&e| values[e as usize].clone())
                    .collect(),
            );
        }
        Ok(())
    }

    /// Keep the sampled-style/layout errors that rendering's automatic motion blur can report,
    /// without calculating transforms, centres, inverse matrices or trail vectors.
    fn trails(
        &mut self,
        at: Tick,
        template: &Rc<Template>,
        values: &[V],
        styles: &[Style],
    ) -> Result<(), Error> {
        let c = self.scene.front.composition;
        let back = c.fps.shutter(360, 0, 2);
        let reach = self.scene.reach(template, styles);
        for offset in [back, -back] {
            let at = Sample {
                offset,
                duration: c.duration,
                looped: c.looped,
            }
            .at(at);
            if let Some(reach) = &reach {
                let then =
                    eval::run_only(&template.program, self.scene.front.clock(at), &reach.read);
                let Some(then) = then.ok().filter(|then| template.holds(then)) else {
                    continue;
                };
                if self
                    .scene
                    .restyled(template, &reach.nodes, &then, values, styles)?
                    .is_some()
                {
                    continue;
                }
            }
            let Some(then) = self.scene.front.values(template, at) else {
                continue;
            };
            self.scene.lay(at, template.clone(), then)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
