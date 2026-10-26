//! Compile the existing Scene3D WGSL kernels for CanvasKit's shared WebGL2 context.
use anyhow::{Context, Result};
use naga::{
    ShaderStage,
    back::glsl,
    valid::{Capabilities, ValidationFlags, Validator},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

const METAL: &str =
    include_str!("../../crates/valle-render/src/executor/skia/scene3d_gpu/metal.wgsl");
const ASSETS: &str =
    include_str!("../../crates/valle-render/src/executor/skia/scene3d_gpu/assets.wgsl");
const LIGHTING: &str =
    include_str!("../../crates/valle-render/src/executor/skia/scene3d_gpu/lighting.wgsl");
const SHADOWS: &str =
    include_str!("../../crates/valle-render/src/executor/skia/scene3d_gpu/shadows.wgsl");
const RASTER: &str =
    include_str!("../../crates/valle-render/src/executor/skia/scene3d_gpu/raster.wgsl");
const TRANSPARENCY: &str =
    include_str!("../../web/packages/engine/src/executor/webgl/scene3d/transparency.wgsl");

fn array_reads(source: &str, array: &str, function: &str) -> Result<String> {
    let marker = format!("{array}[");
    let mut result = String::with_capacity(source.len());
    let mut remaining = source;
    while let Some(at) = remaining.find(&marker) {
        result.push_str(&remaining[..at]);
        let expression = &remaining[at + marker.len()..];
        let mut depth = 1;
        let mut end = None;
        for (index, character) in expression.char_indices() {
            match character {
                '[' => depth += 1,
                ']' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                end = Some(index);
                break;
            }
        }
        let end = end.context("unterminated Scene3D storage read")?;
        result.push_str(function);
        result.push('(');
        result.push_str(&expression[..end]);
        result.push(')');
        remaining = &expression[end + 1..];
    }
    result.push_str(remaining);
    Ok(result)
}

fn source() -> Result<String> {
    let source = format!("{METAL}\n{ASSETS}\n{LIGHTING}\n{SHADOWS}\n{RASTER}\n{TRANSPARENCY}");
    let source = source.replace(
        "var<storage, read> shading: array<vec4<f32>>;",
        "var shadingData: texture_2d<f32>;\nfn shading_at(index:u32)->vec4<f32>{let width=textureDimensions(shadingData).x;return textureLoad(shadingData,vec2<i32>(i32(index%width),i32(index/width)),0);}",
    ).replace(
        "var<storage,read> rasterVertices:array<u32>;",
        "var vertexData:texture_2d<u32>;\nfn vertex_word(index:u32)->u32{let width=textureDimensions(vertexData).x;let at=index/4u;return textureLoad(vertexData,vec2<i32>(i32(at%width),i32(at/width)),0)[index%4u];}",
    ).replace("shadowMaps:texture_2d_array<f32>", "shadowMaps:texture_2d<f32>")
        .replace("textureLoad(shadowMaps,at,layer,0)", "textureLoad(shadowMaps,at+vec2(0,layer*1024),0)")
        .replace("raster_bary(base,7u,in.position.xy)", "raster_bary(base,7u,in.position.xy-vec2(0.0,shadowRange.z))");
    // The edge conversion scales a positive integer mantissa by a small power of two.
    // WebGL2's ES 3.00 lacks ldexp; this exact binary scaling needs no approximation.
    let source = source.replace(
        "ldexp(f32(mantissa),i32(shift))",
        "f32(mantissa)*exp2(f32(shift))",
    );
    array_reads(
        &array_reads(&source, "shading", "shading_at")?,
        "rasterVertices",
        "vertex_word",
    )
}

fn compile(source: &str, stage: ShaderStage, entry: &str) -> Result<Value> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|error| anyhow::anyhow!(error.emit_to_string(source)))?;
    let info = Validator::new(
        ValidationFlags::all(),
        Capabilities::default() | Capabilities::SHADER_FLOAT16_IN_FLOAT32,
    )
    .validate(&module)?;
    let options = glsl::Options {
        version: glsl::Version::Embedded {
            version: 300,
            is_webgl: true,
        },
        ..Default::default()
    };
    let pipeline = glsl::PipelineOptions {
        shader_stage: stage,
        entry_point: entry.into(),
        multiview: None,
    };
    let mut output = String::new();
    let mut writer = glsl::Writer::new(
        &mut output,
        &module,
        &info,
        &options,
        &pipeline,
        naga::proc::BoundsCheckPolicies::default(),
    )?;
    let reflection = writer.write()?;
    let mut textures = BTreeMap::new();
    let mut blocks = BTreeMap::new();
    for (name, mapping) in reflection.texture_mapping {
        let binding = module.global_variables[mapping.texture]
            .binding
            .context("Scene3D texture binding missing")?;
        textures.insert(name, binding.group * 8 + binding.binding);
    }
    for (handle, name) in reflection.uniforms {
        let variable = &module.global_variables[handle];
        if variable.space == naga::AddressSpace::Uniform {
            let binding = variable
                .binding
                .context("Scene3D uniform binding missing")?;
            blocks.insert(name, binding.group * 8 + binding.binding);
        }
    }
    Ok(json!({"source":output,"textures":textures,"blocks":blocks}))
}

fn generated() -> Result<String> {
    let source = source()?;
    let mut programs = BTreeMap::new();
    for (name, vertex, fragment) in [
        ("unlit", "raster_vertex", "raster_fragment"),
        ("lit", "raster_lit_vertex", "raster_lit_fragment"),
        ("shadow", "shadow_vertex", "shadow_fragment"),
        ("background", "background_vertex", "background_fragment"),
        ("transparency", "background_vertex", "web_transparency"),
    ] {
        programs.insert(name, json!({"vertex":compile(&source, ShaderStage::Vertex, vertex)?,"fragment":compile(&source, ShaderStage::Fragment, fragment)?}));
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&programs)?))
}

pub(super) fn generate(root: &Path) -> Result<()> {
    let destination = root.join("web/packages/engine/src/executor/webgl/scene3d/shaders.json");
    fs::create_dir_all(destination.parent().unwrap())?;
    fs::write(destination, generated()?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn web_scene3d_shaders_match_the_shared_native_kernels() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let expected = generated()?;
        let actual = fs::read_to_string(
            root.join("web/packages/engine/src/executor/webgl/scene3d/shaders.json"),
        )?;
        if actual != expected {
            anyhow::bail!("Scene3D shaders changed; run cargo xtask web-shaders");
        }
        Ok(())
    }
}
