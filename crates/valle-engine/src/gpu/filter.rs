//! SVG filter graphs share intermediate GPU targets and release them after their last reader.
use super::Step;
use super::plan::{Plan, Reach};
use crate::layout::Rect;
use crate::paint::Matrix;
use crate::svg::filter::{Graph, Operation, Transfer};

#[derive(Clone, Copy)]
struct Pass {
    linear: bool,
    region: [f64; 4],
    matrix: Matrix,
}

impl Plan {
    fn graph_pass(
        &mut self,
        inputs: [usize; 2],
        op: u32,
        pass: Pass,
        parameters: &[f32],
        data: &[f32],
    ) -> usize {
        let Pass {
            linear,
            region,
            matrix,
        } = pass;
        let transform = self
            .transform(matrix)
            .expect("a planned SVG transform is invertible");
        let mut header = [0.0; 20];
        header[..4].copy_from_slice(&[op as f32, u8::from(linear) as f32, transform as f32, 0.0]);
        header[4..8].copy_from_slice(&region.map(|v| v as f32));
        header[8..8 + parameters.len()].copy_from_slice(parameters);
        let at = self.words(header);
        for chunk in data.chunks(20) {
            let mut words = [0.0; 20];
            words[..chunk.len()].copy_from_slice(chunk);
            self.words(words);
        }
        let target = self.layer();
        self.bounds[target] = Reach::of(matrix, rect(region));
        self.steps.push(Step::FilterGraph { target, inputs, at });
        target
    }

    /// Two separable passes, with axes transformed into target pixels. Both passes work in
    /// the primitive's color space; only the final pass clips to the primitive subregion.
    fn graph_kernel(
        &mut self,
        mut layer: usize,
        kernel: (u32, bool),
        amounts: [f64; 2],
        pass: Pass,
        filter_region: [f64; 4],
    ) -> usize {
        let (op, dilate) = kernel;
        let Pass {
            linear,
            region,
            matrix,
        } = pass;
        for (axis, amount) in amounts.into_iter().enumerate() {
            let v = matrix.0[axis];
            let scale = v[0].hypot(v[1]);
            let direction = if scale > 0.0 {
                [(v[0] / scale) as f32, (v[1] / scale) as f32]
            } else {
                [0.0; 2]
            };
            let params = [
                direction[0],
                direction[1],
                (amount * scale) as f32,
                u8::from(dilate) as f32,
            ];
            layer = self.graph_pass(
                [layer; 2],
                op,
                Pass {
                    linear,
                    region: if axis == 0 { filter_region } else { region },
                    matrix,
                },
                &params,
                &[],
            );
        }
        layer
    }

    pub(super) fn svg_filter(
        &mut self,
        source: usize,
        graph: &Graph,
        images: &[(usize, usize)],
        bbox: [f64; 4],
        viewport: [f64; 2],
        matrix: Matrix,
    ) -> (usize, Reach) {
        let regions = graph.regions(bbox, viewport);
        let filter_region = regions[0];
        let blank = self.layer();
        self.bounds[blank] = Reach::default();
        self.steps.push(Step::Draw {
            target: blank,
            runs: Vec::new(),
        });
        let mut layers = vec![blank; graph.nodes.len() + 2];
        layers[0] = self.graph_pass(
            [source; 2],
            0,
            Pass {
                linear: false,
                region: filter_region,
                matrix,
            },
            &[],
            &[],
        );
        // Only ancestors of the final result participate, even if other result names exist.
        let needed = graph.needed();
        if needed[1] {
            layers[1] = self.graph_pass(
                [layers[0]; 2],
                1,
                Pass {
                    linear: false,
                    region: filter_region,
                    matrix,
                },
                &[],
                &[],
            );
        }
        let units = if graph.definition.primitive_user_space {
            [1.0; 2]
        } else {
            [bbox[2], bbox[3]]
        };
        let offset = |v: [f64; 2]| {
            let [x, y] = [v[0] * units[0], v[1] * units[1]];
            [
                (matrix.0[0][0] * x + matrix.0[1][0] * y) as f32,
                (matrix.0[0][1] * x + matrix.0[1][1] * y) as f32,
            ]
        };
        for (i, n) in graph.nodes.iter().enumerate() {
            if !needed[i + 2] {
                continue;
            }
            let region = regions[i + 2];
            let a = n.inputs.first().map_or(blank, |i| layers[*i]);
            let b = n.inputs.get(1).map_or(a, |i| layers[*i]);
            let linear = n.linear;
            let layer = match &n.op {
                Operation::Turbulence(noise) => {
                    let origin = if graph.definition.primitive_user_space {
                        [0.; 2]
                    } else {
                        [bbox[0], bbox[1]]
                    };
                    let tile = [
                        (region[0] - origin[0]) / units[0],
                        (region[1] - origin[1]) / units[1],
                        region[2] / units[0],
                        region[3] / units[1],
                    ];
                    let (frequency, stitch) = noise.stitch(tile);
                    let parameters = [
                        frequency[0] as f32,
                        frequency[1] as f32,
                        noise.octaves as f32,
                        u8::from(noise.fractal) as f32,
                        origin[0] as f32,
                        origin[1] as f32,
                        units[0] as f32,
                        units[1] as f32,
                        stitch[0],
                        stitch[1],
                        stitch[2],
                        stitch[3],
                    ];
                    self.graph_pass(
                        [blank; 2],
                        13,
                        Pass {
                            linear,
                            region,
                            matrix,
                        },
                        &parameters,
                        &noise.table,
                    )
                }
                Operation::Convolve(kernel) => {
                    let pixel = [0, 1].map(|axis| 1. / matrix.0[axis][0].hypot(matrix.0[axis][1]));
                    let unit = kernel
                        .unit
                        .map_or(pixel, |u| [u[0] * units[0], u[1] * units[1]]);
                    let mut data = regions[n.inputs[0]].map(|v| v as f32).to_vec();
                    data.extend([pixel[0] as f32, pixel[1] as f32]);
                    data.extend(kernel.values.iter().map(|v| *v as f32));
                    let params = [
                        kernel.order[0] as f32,
                        kernel.order[1] as f32,
                        kernel.target[0] as f32,
                        kernel.target[1] as f32,
                        unit[0] as f32,
                        unit[1] as f32,
                        kernel.divisor as f32,
                        kernel.bias as f32,
                        0.,
                        0.,
                        kernel.edge as f32,
                        u8::from(kernel.preserve_alpha) as f32,
                    ];
                    self.graph_pass(
                        [a; 2],
                        14,
                        Pass {
                            linear,
                            region,
                            matrix,
                        },
                        &params,
                        &data,
                    )
                }
                Operation::Lighting(lighting) => {
                    let pixel = [0, 1].map(|axis| 1. / matrix.0[axis][0].hypot(matrix.0[axis][1]));
                    let unit = lighting
                        .unit
                        .map_or(pixel, |u| [u[0] * units[0], u[1] * units[1]]);
                    let z = units[0].hypot(units[1]) / std::f64::consts::SQRT_2;
                    let origin = if graph.definition.primitive_user_space {
                        [0.; 2]
                    } else {
                        [bbox[0], bbox[1]]
                    };
                    let mut data = regions[n.inputs[0]].map(|v| v as f32).to_vec();
                    data.extend(
                        lighting
                            .light
                            .as_ref()
                            .expect("validated light source")
                            .parameters(origin, [units[0], units[1], z]),
                    );
                    let c = lighting.color;
                    let params = [
                        unit[0] as f32,
                        unit[1] as f32,
                        (lighting.surface * z) as f32,
                        lighting.constant as f32,
                        c[0] as f32,
                        c[1] as f32,
                        c[2] as f32,
                        lighting.specular.unwrap_or(0.) as f32,
                    ];
                    self.graph_pass(
                        [a; 2],
                        15,
                        Pass {
                            linear,
                            region,
                            matrix,
                        },
                        &params,
                        &data,
                    )
                }
                Operation::Image(_) => {
                    let source = images
                        .iter()
                        .find(|(index, _)| *index == i)
                        .map_or(blank, |(_, layer)| *layer);
                    self.graph_pass(
                        [source; 2],
                        0,
                        Pass {
                            linear: false,
                            region,
                            matrix,
                        },
                        &[],
                        &[],
                    )
                }
                Operation::Tile => self.graph_pass(
                    [a; 2],
                    12,
                    Pass {
                        linear: false,
                        region,
                        matrix,
                    },
                    &regions[n.inputs[0]].map(|v| v as f32),
                    &[],
                ),
                Operation::Flood => self.graph_pass(
                    [blank; 2],
                    2,
                    Pass {
                        linear: false,
                        region,
                        matrix,
                    },
                    &n.flood.map(|v| v as f32),
                    &[],
                ),
                Operation::Offset(v) => self.graph_pass(
                    [a; 2],
                    3,
                    Pass {
                        linear: false,
                        region,
                        matrix,
                    },
                    &offset(*v),
                    &[],
                ),
                Operation::Matrix(m) => self.graph_pass(
                    [a; 2],
                    4,
                    Pass {
                        linear,
                        region,
                        matrix,
                    },
                    &[],
                    &m.map(|v| v as f32),
                ),
                Operation::Blend(mode) => self.graph_pass(
                    [a, b],
                    5,
                    Pass {
                        linear,
                        region,
                        matrix,
                    },
                    &[*mode as f32],
                    &[],
                ),
                Operation::Composite { operator, k } => self.graph_pass(
                    [a, b],
                    6,
                    Pass {
                        linear,
                        region,
                        matrix,
                    },
                    &[
                        *operator as f32,
                        0.,
                        0.,
                        0.,
                        k[0] as f32,
                        k[1] as f32,
                        k[2] as f32,
                        k[3] as f32,
                    ],
                    &[],
                ),
                Operation::Merge => {
                    let mut layer = blank;
                    for input in &n.inputs {
                        layer = self.graph_pass(
                            [layers[*input], layer],
                            6,
                            Pass {
                                linear,
                                region,
                                matrix,
                            },
                            &[0.],
                            &[],
                        );
                    }
                    layer
                }
                Operation::Transfer => self.graph_pass(
                    [a; 2],
                    7,
                    Pass {
                        linear,
                        region,
                        matrix,
                    },
                    &[],
                    &transfers(&n.transfer),
                ),
                Operation::Blur(v) => self.graph_kernel(
                    a,
                    (8, false),
                    [v[0] * units[0], v[1] * units[1]],
                    Pass {
                        linear,
                        region,
                        matrix,
                    },
                    filter_region,
                ),
                Operation::Morphology { dilate, radius } => self.graph_kernel(
                    a,
                    (9, *dilate),
                    [radius[0] * units[0], radius[1] * units[1]],
                    Pass {
                        linear,
                        region,
                        matrix,
                    },
                    filter_region,
                ),
                Operation::Displacement { scale, channels } => self.graph_pass(
                    [a, b],
                    10,
                    Pass {
                        linear,
                        region,
                        matrix,
                    },
                    &[
                        (scale * units[0]) as f32,
                        (scale * units[1]) as f32,
                        channels[0] as f32,
                        channels[1] as f32,
                    ],
                    &[],
                ),
                Operation::Shadow { offset: v, sigma } => {
                    let alpha = self.graph_pass(
                        [a; 2],
                        1,
                        Pass {
                            linear: false,
                            region: filter_region,
                            matrix,
                        },
                        &[],
                        &[],
                    );
                    let blurred = self.graph_kernel(
                        alpha,
                        (8, false),
                        [sigma[0] * units[0], sigma[1] * units[1]],
                        Pass {
                            linear: false,
                            region: filter_region,
                            matrix,
                        },
                        filter_region,
                    );
                    let colored = self.graph_pass(
                        [blurred; 2],
                        11,
                        Pass {
                            linear: false,
                            region: filter_region,
                            matrix,
                        },
                        &n.flood.map(|v| v as f32),
                        &[],
                    );
                    let moved = self.graph_pass(
                        [colored; 2],
                        3,
                        Pass {
                            linear: false,
                            region: filter_region,
                            matrix,
                        },
                        &offset(*v),
                        &[],
                    );
                    self.graph_pass(
                        [a, moved],
                        6,
                        Pass {
                            linear,
                            region,
                            matrix,
                        },
                        &[0.],
                        &[],
                    )
                }
                Operation::Channel(..) | Operation::MergeNode | Operation::Light(_) => {
                    unreachable!("validated primitive child")
                }
            };
            layers[i + 2] = layer;
        }
        let last = *layers.last().unwrap();
        (last, self.bound(last))
    }
}

fn rect([x, y, w, h]: [f64; 4]) -> Rect {
    Rect {
        x: x as f32,
        y: y as f32,
        width: w as f32,
        height: h as f32,
    }
}

fn transfers(functions: &[Transfer; 4]) -> Vec<f32> {
    let mut data = vec![0.; 16];
    for (i, f) in functions.iter().enumerate() {
        let header = match f {
            Transfer::Identity => [0.; 4],
            Transfer::Linear([a, b]) => [3., *a as f32, *b as f32, 0.],
            Transfer::Gamma([a, b, c]) => [4., *a as f32, *b as f32, *c as f32],
            Transfer::Table(values, discrete) => {
                let start = data.len();
                data.extend(values.iter().map(|v| *v as f32));
                [
                    if *discrete { 2. } else { 1. },
                    start as f32,
                    values.len() as f32,
                    0.,
                ]
            }
        };
        data[i * 4..i * 4 + 4].copy_from_slice(&header);
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::{Gpu, Renderer};
    use crate::paint::{Item, work};
    use crate::svg::{
        Length,
        filter::{Definition, Node},
    };
    use std::rc::Rc;

    fn node(op: Operation, inputs: &[usize]) -> Node {
        Node {
            inputs: inputs.to_vec(),
            region: [None; 4],
            op,
            linear: true,
            flood: [0.; 4],
            transfer: std::array::from_fn(|_| Transfer::Identity),
        }
    }
    fn graph(nodes: Vec<Node>, items: Vec<Item>) -> Item {
        Item::FilterGraph {
            graph: Rc::new(Graph {
                definition: Definition {
                    user_space: true,
                    primitive_user_space: true,
                    region: [0., 0., 16., 16.].map(|units| Length { units, percent: 0. }),
                },
                nodes,
            }),
            images: Vec::new(),
            bbox: [0., 0., 16., 16.],
            viewport: [16., 16.],
            items,
        }
    }
    fn close(pixel: &[u8], expected: [u8; 4]) {
        assert!(
            pixel.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2),
            "{pixel:?} != {expected:?}"
        );
    }

    #[test]
    fn filter_graph_pixels_cover_spaces_aliasing_empty_sources_and_regions() {
        if !std::env::var("VALLE_TEST_LAYERS")
            .unwrap_or_default()
            .split(',')
            .any(|v| v == "gpu")
        {
            return;
        }
        let gpu = valle_native::gpu::adapter(None)
            .and_then(|a| Gpu::with(a.adapter))
            .unwrap();
        let mut renderer = Renderer::new(&gpu, 16, 16);
        let draw = |renderer: &mut Renderer<'_>, nodes, items| {
            renderer
                .render(&[graph(nodes, items)], &[], &[], &[])
                .unwrap()
        };
        // Zero frequency has known RGBA values in either noise mode, including an empty source.
        for fractal in [true, false] {
            let n = node(
                Operation::Turbulence(crate::svg::filter::turbulence::Noise {
                    frequency: [0.; 2],
                    octaves: 3,
                    fractal,
                    stitch: false,
                    table: crate::svg::filter::turbulence::table(7.),
                }),
                &[],
            );
            let pixels = draw(&mut renderer, vec![n], vec![]);
            close(
                &pixels[..4],
                if fractal {
                    [188, 188, 188, 128]
                } else {
                    [0; 4]
                },
            );
        }
        // A front light on a flat height map produces a constant opaque diffuse map,
        // including the Sobel corners and edges. Lighting colors do not contribute alpha.
        let light = crate::svg::filter::lighting::Lighting {
            surface: 2.,
            specular: None,
            constant: 0.5,
            unit: None,
            light: Some(crate::svg::filter::lighting::Light::Distant([0., 90.])),
            color: [1., 0.5, 0.25, 0.],
        };
        // The specular map stores max(RGB) as alpha, unlike opaque diffuse lighting.
        for linear in [false, true] {
            for (elevation, exponent, alpha) in [(90., 12., 128), (0., 2., 64)] {
                let mut specular = light.clone();
                specular.specular = Some(exponent);
                specular.light = Some(crate::svg::filter::lighting::Light::Distant([
                    0., elevation,
                ]));
                let mut highlight = node(Operation::Lighting(specular), &[0]);
                highlight.linear = linear;
                let pixels = draw(&mut renderer, vec![highlight], vec![]);
                close(&pixels[..4], [255, 128, 64, alpha]);
            }
        }
        let mut diffuse = node(Operation::Lighting(light.clone()), &[0]);
        diffuse.linear = false;
        let pixels = draw(&mut renderer, vec![diffuse], vec![]);
        for pixel in pixels.as_chunks::<4>().0 {
            close(pixel, [128, 64, 32, 255]);
        }
        let mut light = light;
        light.constant = 1.;
        light.color = [1.; 4];
        light.light = Some(crate::svg::filter::lighting::Light::Distant([180., 0.]));
        let mut diffuse = node(Operation::Lighting(light), &[0]);
        diffuse.linear = false;
        let step = Item::Rect {
            rect: rect([8., 0., 8., 16.]),
            radii: [[0.; 2]; 4],
            color: work([1.; 4]),
        };
        let pixels = draw(&mut renderer, vec![diffuse], vec![step]);
        close(
            &pixels[(8 * 16 + 7) * 4..(8 * 16 + 8) * 4],
            [228, 228, 228, 255],
        );
        let light = crate::svg::filter::lighting::Lighting {
            surface: 1.,
            specular: None,
            constant: 1.,
            unit: None,
            color: [1.; 4],
            light: Some(crate::svg::filter::lighting::Light::Spot {
                position: [200., 0., 30.],
                target: [200., 0., 0.],
                exponent: 1.,
                cone: Some(20.),
            }),
        };
        let pixels = draw(
            &mut renderer,
            vec![node(Operation::Lighting(light), &[0])],
            vec![],
        );
        for pixel in pixels.as_chunks::<4>().0 {
            close(pixel, [0, 0, 0, 255]);
        }
        // Flood has no input dependency. The graph must execute even with no drawing runs.
        let mut red = node(Operation::Flood, &[]);
        red.flood = [1., 0., 0., 0.5];
        let mut blue = node(Operation::Flood, &[]);
        blue.flood = [0., 0., 1., 0.5];
        for linear in [false, true, false] {
            let mut over = node(
                Operation::Composite {
                    operator: 0,
                    k: [0.; 4],
                },
                &[2, 3],
            );
            over.linear = linear;
            let pixels = draw(&mut renderer, vec![red.clone(), blue.clone(), over], vec![]);
            close(
                &pixels[0..4],
                if linear {
                    [213, 0, 156, 191]
                } else {
                    [170, 0, 85, 191]
                },
            );
        }
        // The same target can feed both inputs and later frames reuse its allocation safely.
        let alias = node(
            Operation::Composite {
                operator: 0,
                k: [0.; 4],
            },
            &[2, 2],
        );
        for _ in 0..3 {
            let pixels = draw(&mut renderer, vec![red.clone(), alias.clone()], vec![]);
            close(&pixels[0..4], [255, 0, 0, 191]);
        }
        // Preserve-alpha convolves straight RGB and applies bias before repremultiplication.
        let kernel = crate::svg::filter::convolve::Kernel {
            order: [1, 1],
            values: vec![1.],
            divisor: 1.,
            bias: 0.25,
            target: [0, 0],
            edge: 0,
            unit: None,
            preserve_alpha: true,
        };
        let mut convolve = node(Operation::Convolve(kernel), &[2]);
        convolve.linear = false;
        let pixels = draw(&mut renderer, vec![red.clone(), convolve], vec![]);
        close(&pixels[..4], [255, 64, 64, 128]);
        // Arithmetic's constant term can create visible pixels from a transparent source.
        let constant = node(
            Operation::Composite {
                operator: 5,
                k: [0., 0., 0., 0.25],
            },
            &[0, 0],
        );
        let pixels = draw(&mut renderer, vec![constant], vec![]);
        close(&pixels[0..4], [255, 255, 255, 64]);
        red.region = [
            Some(Length {
                units: 4.,
                percent: 0.,
            }),
            Some(Length {
                units: 4.,
                percent: 0.,
            }),
            Some(Length {
                units: 4.,
                percent: 0.,
            }),
            Some(Length {
                units: 4.,
                percent: 0.,
            }),
        ];
        let pixels = draw(&mut renderer, vec![red.clone()], vec![]);
        assert_eq!(&pixels[0..4], &[0; 4]);
        close(
            &pixels[(5 * 16 + 5) * 4..(5 * 16 + 5) * 4 + 4],
            [255, 0, 0, 128],
        );
        // A reversed asymmetric kernel moves the 4..8 input two pixels left.
        let kernel = crate::svg::filter::convolve::Kernel {
            order: [3, 1],
            values: vec![1., 0., 0.],
            divisor: 1.,
            bias: 0.,
            target: [0, 0],
            edge: 2,
            unit: None,
            preserve_alpha: false,
        };
        let mut convolve = node(Operation::Convolve(kernel), &[2]);
        convolve.region = [0., 0., 16., 16.].map(|units| Some(Length { units, percent: 0. }));
        let pixels = draw(&mut renderer, vec![red.clone(), convolve], vec![]);
        close(
            &pixels[(4 * 16 + 2) * 4..(4 * 16 + 3) * 4],
            [255, 0, 0, 128],
        );
        close(&pixels[(4 * 16 + 6) * 4..(4 * 16 + 7) * 4], [0; 4]);
        // Fractional taps wrap at the input subregion, including both sides of a bilinear sample.
        let kernel = crate::svg::filter::convolve::Kernel {
            order: [2, 1],
            values: vec![1., 0.],
            divisor: 1.,
            bias: 0.,
            target: [0, 0],
            edge: 1,
            unit: Some([0.5, 1.]),
            preserve_alpha: false,
        };
        let pixels = draw(
            &mut renderer,
            vec![red.clone(), node(Operation::Convolve(kernel), &[2])],
            vec![],
        );
        close(
            &pixels[(4 * 16 + 7) * 4..(4 * 16 + 8) * 4],
            [255, 0, 0, 128],
        );
        // Tile defaults to the whole filter, not its smaller input subregion.
        let tile = node(Operation::Tile, &[2]);
        let pixels = draw(&mut renderer, vec![red.clone(), tile.clone()], vec![]);
        for pixel in pixels.as_chunks::<4>().0 {
            close(pixel, [255, 0, 0, 128]);
        }
        red.region[2].as_mut().unwrap().units = 0.;
        let pixels = draw(&mut renderer, vec![red, tile], vec![]);
        assert!(pixels.iter().all(|v| *v == 0), "an empty tile stays empty");
        let source = Item::Rect {
            rect: rect([2., 2., 8., 8.]),
            radii: [[0.; 2]; 4],
            color: work([0., 1., 0., 1.]),
        };
        let pixels = draw(&mut renderer, vec![], vec![source.clone()]);
        assert!(
            pixels.iter().all(|v| *v == 0),
            "empty definitions hide the source"
        );
        let pixels = draw(
            &mut renderer,
            vec![node(Operation::Offset([2., 3.]), &[0])],
            vec![source],
        );
        close(
            &pixels[(6 * 16 + 5) * 4..(6 * 16 + 5) * 4 + 4],
            [0, 255, 0, 255],
        );
        assert_eq!(&pixels[(3 * 16 + 3) * 4..(3 * 16 + 3) * 4 + 4], &[0; 4]);
    }
}
