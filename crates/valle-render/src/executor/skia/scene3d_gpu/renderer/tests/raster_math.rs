use super::*;
use std::{sync::mpsc, time::Duration};
use wgpu::util::DeviceExt;

#[test]
#[ignore = "requires a real GPU; verifies full-width signed edges and f32 rounding against Rust i64"]
fn integer_edges_and_rounding_match_cpu_at_large_coordinates() {
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
    let edge=edge64(a,b,p);var value=0.0;
    if i32(edge.y)>=0 {value=positive_float64(edge);}
    mathResults[id.x]=vec4(edge,bitcast<u32>(value),select(0u,1u,edge_inside(edge,a,b)));
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
        let bits = edge as u64;
        let inside = edge > 0 || (edge == 0 && (by > ay || (by == ay && bx < ax)));
        let expected = [
            bits as u32,
            (bits >> 32) as u32,
            if edge >= 0 {
                (edge as f32).to_bits()
            } else {
                0
            },
            u32::from(inside),
        ];
        let actual: [u32; 4] = std::array::from_fn(|i| {
            u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
        });
        assert_eq!(actual, expected, "edge inputs={record:?}");
    }
}
