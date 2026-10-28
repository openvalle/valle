use super::*;
use std::{sync::mpsc, time::Duration};
use wgpu::util::DeviceExt;

#[test]
#[ignore = "requires a real GPU; verifies floating interpolation over viewport-sized coordinates"]
fn floating_edges_bound_interpolation_error_at_large_coordinates() {
    let (renderer, _context) = renderer();
    let device = &renderer.device;
    let queue = &renderer.queue;
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        ((state % 4_194_305) as i32) - 2_097_152
    };
    let records = (0..4096)
        .map(|i| {
            if i < 4 {
                [
                    [0, 0, 256, 0, 128, 0],
                    [256, 0, 0, 0, 128, 0],
                    [0, 0, 0, 256, 0, 128],
                    [0, 0, 0, -256, 0, -128],
                ][i]
            } else {
                std::array::from_fn(|_| next())
            }
        })
        .collect::<Vec<[i32; 6]>>();
    let source = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}",
        include_str!("../../common.wgsl"),
        include_str!("../../assets.wgsl"),
        include_str!("../../lighting.wgsl"),
        include_str!("../../shadows.wgsl"),
        include_str!("../../transparency.wgsl"),
        include_str!("../../raster.wgsl"),
        r#"
@group(0) @binding(7) var<storage,read_write> mathResults:array<vec4<u32>>;
@compute @workgroup_size(64) fn validate_math(@builtin(global_invocation_id) id:vec3<u32>) {
    let base=id.x*6u;
    let a=vec2<i32>(i32(rasterVertices[base]),i32(rasterVertices[base+1u]));
    let b=vec2<i32>(i32(rasterVertices[base+2u]),i32(rasterVertices[base+3u]));
    let p=vec2<i32>(i32(rasterVertices[base+4u]),i32(rasterVertices[base+5u]));
    let edge=raster_edge(vec2<f32>(a)/256.0,vec2<f32>(b)/256.0,vec2<f32>(p)/256.0);
    mathResults[id.x]=vec4(bitcast<u32>(edge),0u,0u,0u);
}
"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production Scene3D edge math"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("edge math validation"),
        layout: None,
        module: &module,
        entry_point: Some("validate_math"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bytes = records
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>();
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: &bytes,
        usage: wgpu::BufferUsages::STORAGE,
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 4096 * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 4096 * 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 6,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(64, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 4096 * 16);
    let submitted = queue.submit([encoder.finish()]);
    let (send, receive) = mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submitted),
            timeout: Some(Duration::from_secs(30)),
        })
        .unwrap();
    receive.recv().unwrap().unwrap();
    let data = readback.slice(..).get_mapped_range().unwrap();
    for (record, bytes) in records.iter().zip(data.as_chunks::<16>().0) {
        let [ax, ay, bx, by, px, py] = record.map(i64::from);
        let edge = (bx - ax) * (py - ay) - (by - ay) * (px - ax);
        let expected = edge as f64 / 65536.0;
        let actual = f32::from_le_bytes(bytes[..4].try_into().unwrap()) as f64;
        let magnitude =
            (((bx - ax) * (py - ay)).abs() + ((by - ay) * (px - ax)).abs()) as f64 / 65536.0;
        assert!(
            (actual - expected).abs() <= magnitude * 2e-7 + 1e-6,
            "edge inputs={record:?}, {actual}/{expected}"
        );
    }
}
