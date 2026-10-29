//! Both particle entry points lower to the same bounded scene object.
use super::*;
use serde::Deserialize;
use valle_motion::{particles3d::ParticleSystem3D, scene3d::*};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    seed: u64,
    count: u32,
    emitter: Emitter,
    birth: Birth,
    lifetime: f64,
    velocity: Velocity,
    #[serde(default)]
    gravity: [f32; 3],
    #[serde(default)]
    drag: f64,
    #[serde(default, rename = "loop")]
    looping: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Emitter {
    min: [f32; 3],
    max: [f32; 3],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Birth {
    interval: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Velocity {
    x: [f32; 2],
    y: [f32; 2],
    z: [f32; 2],
}

impl<'s> Compiler<'s> {
    pub(super) fn lower_particles3d(
        &mut self,
        element: &'s JSXElement<'s>,
        outer_key: &str,
        parents: Vec<Scene3DTransformBinding>,
        standalone: bool,
    ) -> Option<(MeshSpec, Scene3DMeshBinding)> {
        let mut allowed = vec![
            "key",
            "positions",
            "shape",
            "sizes",
            "fills",
            "opacities",
            "rotations",
            "material",
            "position",
            "rotation",
            "scale",
            "translateX",
            "translateY",
            "translateZ",
            "rotateX",
            "rotateY",
            "rotateZ",
            "scaleX",
            "scaleY",
            "scaleZ",
        ];
        if standalone {
            allowed.extend(["camera", "pbr", "style", "className", "visible"]);
        }
        let attrs = self.scene3d_attributes(element, "Particles3D", &allowed)?;
        let key = if standalone {
            outer_key.to_owned()
        } else {
            self.scene3d_required_attr_string(&attrs, "key", "Particles3D")?
        };
        let Some((value, span)) = attrs.get("positions") else {
            self.illegal(
                DiagCode::GrammarForbidden,
                element.span(),
                "Particles3D requires positions={particles3d(seconds, options)}",
            );
            return None;
        };
        let expression = self.attr_batch_expression(value, *span, "positions")?;
        let Expression::CallExpression(call) = strip_parens(expression) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                *span,
                "Particles3D positions must call particles3d(seconds, options)",
            );
            return None;
        };
        if !matches!(&call.callee, Expression::Identifier(id) if id.name == "particles3d")
            || call.arguments.len() != 2
            || call.arguments.iter().any(|a| a.as_expression().is_none())
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                *span,
                "Particles3D positions requires particles3d(seconds, options) with two arguments",
            );
            return None;
        }
        let time = self.number_binding(
            call.arguments[0].as_expression()?,
            "particles3d time",
            |t| t.abs() <= 1.0e9,
        )?;
        let options = call.arguments[1].as_expression()?;
        let Some(value) = self.eval_static(options) else {
            self.illegal(
                DiagCode::BuiltinRejected,
                options.span(),
                "particles3d options must be prepare-time data",
            );
            return None;
        };
        let options: Options = match serde_json::from_value(value) {
            Ok(options) => options,
            Err(error) => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    options.span(),
                    format!("invalid particles3d options: {error}"),
                );
                return None;
            }
        };
        let shape = if attrs.contains_key("shape") {
            match self
                .scene3d_required_attr_string(&attrs, "shape", "Particles3D")?
                .as_str()
            {
                "circle" => ParticleShape3D::Circle,
                "quad" => ParticleShape3D::Quad,
                _ => {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        element.span(),
                        "Particles3D shape must be circle or quad",
                    );
                    return None;
                }
            }
        } else {
            ParticleShape3D::Circle
        };
        let mut spec = Particles3DSpec {
            motion: ParticleSystem3D {
                seed: options.seed,
                count: options.count,
                min: options.emitter.min,
                max: options.emitter.max,
                velocity: [options.velocity.x, options.velocity.y, options.velocity.z],
                gravity: options.gravity,
                birth_interval: options.birth.interval,
                lifetime: options.lifetime,
                drag: options.drag,
                looping: options.looping,
            },
            shape,
            sizes: [0.05; 2],
            colors: [Color4([1.0; 4]); 2],
            opacities: [1.0; 2],
            rotations: [0.0; 2],
        };
        for (name, output) in [
            ("sizes", &mut spec.sizes),
            ("opacities", &mut spec.opacities),
            ("rotations", &mut spec.rotations),
        ] {
            if let Some((value, span)) = attrs.get(name) {
                let expression = self.attr_batch_expression(value, *span, name)?;
                let value = self.eval_static(expression);
                let pair = value.as_ref().and_then(|v| {
                    if let Some(n) = v.as_f64() {
                        Some([n as f32; 2])
                    } else {
                        let a = v.as_array()?;
                        (a.len() == 2).then_some(())?;
                        Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32])
                    }
                });
                let Some(pair) = pair else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        *span,
                        format!(
                            "Particles3D {name} must be a static number or [birth, death] pair"
                        ),
                    );
                    return None;
                };
                *output = pair;
            }
        }
        if let Some((value, span)) = attrs.get("fills") {
            let (colors, field) = self.attr_batch_fills(value, *span)?;
            if field.is_some() || !(1..=2).contains(&colors.len()) {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    *span,
                    "Particles3D fills must be a static color or [birth, death] pair",
                );
                return None;
            }
            spec.colors = [
                Color4(colors[0].to_srgb_straight()),
                Color4(colors[colors.len() - 1].to_srgb_straight()),
            ];
        }
        if let Err(error) = spec.validate() {
            self.illegal(
                DiagCode::GrammarForbidden,
                element.span(),
                error.to_string(),
            );
            return None;
        }
        let (mut material, material_frame) = self.scene3d_material(&attrs, element.span())?;
        material.kind.get_or_insert(MaterialKind::Unlit);
        material.alpha_mode.get_or_insert(AlphaMode::Blend);
        material.double_sided.get_or_insert(true);
        let transform = self.scene3d_mesh_transform(&attrs, element.span())?;
        Some((
            MeshSpec {
                key: key.clone(),
                particles: Some(spec),
                model_control: None,
                geometry: None,
                material,
                material_overrides: Vec::new(),
                animation_clip: None,
                node_ids: Vec::new(),
            },
            Scene3DMeshBinding {
                key,
                parents,
                particle_time: Some(time),
                material: material_frame,
                transform,
                material_overrides: Vec::new(),
                animation_time: None,
                nodes: Vec::new(),
            },
        ))
    }
}
