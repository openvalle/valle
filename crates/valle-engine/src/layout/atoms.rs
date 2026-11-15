//! Independent boxes measured by Taffy and placed by the paragraph line breaker.
use super::*;
use crate::text::Atom;
use taffy::prelude::TaffyMaxContent;

impl Tree<'_> {
    /// Inline-blocks export their last in-flow line, unlike flex and grid's first baseline.
    fn last_inline_baseline(&mut self, i: usize) -> Result<Option<f32>, Error> {
        if self.styles[i].layout.display == Display::None {
            return Ok(None);
        }
        let l = self.layouts[i];
        if let Some(c) = &self.contents[i] {
            let width =
                l.size.width - l.padding.left - l.padding.right - l.border.left - l.border.right;
            return Ok(self
                .text
                .lines(c, width)?
                .baselines
                .last()
                .map(|b| b + l.padding.top + l.border.top));
        }
        for child in self.children[i].clone().into_iter().rev() {
            let c = usize::from(child);
            if !self.styles[c].layout.position.is_out_of_flow()
                && let Some(b) = self.last_inline_baseline(c)?
            {
                return Ok(Some(self.layouts[c].location.y + b));
            }
        }
        Ok(None)
    }
    pub(super) fn inline_leaf(&mut self, i: usize, inputs: LayoutInput) -> LayoutOutput {
        let layout = self.style(NodeId::from(i)).clone();
        let calcs = self.calcs.clone();
        let calc = |h, basis| calcs[math::index(h)].resolve(basis);
        let vertical_edges = layout
            .padding
            .top
            .resolve_or_zero(inputs.parent_size.width, calc)
            + layout
                .padding
                .bottom
                .resolve_or_zero(inputs.parent_size.width, calc)
            + layout
                .border
                .top
                .resolve_or_zero(inputs.parent_size.width, calc)
            + layout
                .border
                .bottom
                .resolve_or_zero(inputs.parent_size.width, calc);
        let height = layout
            .size
            .height
            .maybe_resolve(inputs.parent_size.height, calc)
            .map(|h| {
                if layout.box_sizing == BoxSizing::BorderBox {
                    (h - vertical_edges).max(0.0)
                } else {
                    h
                }
            });
        let mut content = self.contents[i].clone().expect("text content");
        let mut out = compute_leaf_layout(inputs, &layout, calc, |known, available| {
            let space = known
                .width
                .map_or(available.width, AvailableSpace::Definite);
            self.inline_sizes(&mut content, space, height);
            let basis = known.width.unwrap_or(match space {
                AvailableSpace::Definite(w) => w,
                _ => 0.0,
            });
            let resolved = content.with_inline_basis(basis);
            let c = resolved.as_ref().unwrap_or(&content);
            let result = match space {
                AvailableSpace::Definite(w) => self.text.measure(c, Some(w)),
                AvailableSpace::MinContent => self.text.min_content(c),
                AvailableSpace::MaxContent => self.text.measure(c, None),
            };
            match result {
                Ok([w, h]) => Size {
                    width: known.width.unwrap_or(w),
                    height: known.height.unwrap_or(h),
                },
                Err(e) => {
                    self.failed.get_or_insert(e);
                    Size::ZERO
                }
            }
        });
        if inputs.run_mode == RunMode::PerformLayout {
            let side = |l: LengthPercentage| l.resolve_or_zero(inputs.parent_size.width, calc);
            let padding = layout.padding.map(side);
            let border = layout.border.map(side);
            let width = out.size.width - padding.left - padding.right - border.left - border.right;
            self.inline_sizes(&mut content, AvailableSpace::Definite(width), height);
            match self.text.lines(&content, width) {
                Ok(lines) => {
                    let top = padding.top + border.top;
                    out.baselines.first = lines.baselines.first().map(|b| top + b);
                    out.baselines.last = lines.baselines.last().map(|b| top + b);
                }
                Err(e) => {
                    self.failed.get_or_insert(e);
                }
            }
            self.contents[i] = Some(content);
        }
        out
    }

    fn inline_sizes(
        &mut self,
        content: &mut Content<'_>,
        space: AvailableSpace,
        height: Option<f32>,
    ) {
        for p in &mut content.pieces {
            if p.atom.is_none() {
                continue;
            }
            let id = NodeId::from(p.node as usize);
            let style = self.style(id).clone();
            let basis = match space {
                AvailableSpace::Definite(w) => Some(w),
                _ => None,
            };
            let margin = style
                .margin
                .map(|m| m.resolve_or_zero(basis, |h, b| self.calc(h, b)));
            let padding = style
                .padding
                .map(|m| m.resolve_or_zero(basis, |h, b| self.calc(h, b)));
            let border = style
                .border
                .map(|m| m.resolve_or_zero(basis, |h, b| self.calc(h, b)));
            let input = LayoutInput {
                run_mode: RunMode::ComputeSize,
                parent_size: Size {
                    width: basis,
                    height,
                },
                available_space: Size::MAX_CONTENT,
                ..LayoutInput::HIDDEN
            };
            let max = self.compute(id, input, None).size.width;
            let width = if style.size.width.is_auto() {
                let min = self
                    .compute(
                        id,
                        LayoutInput {
                            available_space: Size {
                                width: AvailableSpace::MinContent,
                                height: AvailableSpace::MaxContent,
                            },
                            ..input
                        },
                        None,
                    )
                    .size
                    .width;
                match space {
                    AvailableSpace::MinContent => min,
                    AvailableSpace::MaxContent => max,
                    AvailableSpace::Definite(w) => {
                        max.min((w - margin.left - margin.right).max(min))
                    }
                }
            } else {
                max
            };
            let out = self.compute(
                id,
                LayoutInput {
                    run_mode: RunMode::PerformLayout,
                    known_dimensions: Size {
                        width: Some(width),
                        height: None,
                    },
                    available_space: Size {
                        width: AvailableSpace::Definite(width),
                        height: AvailableSpace::MaxContent,
                    },
                    ..input
                },
                None,
            );
            let l = taffy::Layout {
                size: out.size,
                padding,
                border,
                margin,
                ..taffy::Layout::with_order(0)
            };
            self.set_unrounded_layout(id, &l);
            let height = out.size.height + margin.top + margin.bottom;
            let vertical = writing::vertical(content.font.writing);
            let baseline = if vertical {
                let m = self.text.strut(content.font).unwrap_or_else(|e| {
                    self.failed.get_or_insert(e);
                    [0.0; 2]
                });
                height * 0.5 + (m[0] - m[1]) * 0.5
            } else if style.overflow.y != Overflow::Visible
                || self.replaced[p.node as usize].is_some()
            {
                height
            } else {
                let b = match style.display {
                    Display::Block => {
                        self.last_inline_baseline(p.node as usize)
                            .unwrap_or_else(|e| {
                                self.failed.get_or_insert(e);
                                None
                            })
                    }
                    _ => out.baselines.first,
                };
                b.map_or(height, |b| margin.top + b)
            };
            let inset = style.inset;
            let shift = if style.position == Position::Relative {
                let left = inset.left.maybe_resolve(basis, |h, b| self.calc(h, b));
                let right = inset.right.maybe_resolve(basis, |h, b| self.calc(h, b));
                let x = if style.direction == Direction::Rtl {
                    right.map(|r| -r).or(left)
                } else {
                    left.or(right.map(|r| -r))
                };
                [
                    x.unwrap_or(0.0),
                    inset
                        .top
                        .maybe_resolve(input.parent_size.height, |h, b| self.calc(h, b))
                        .or_else(|| {
                            inset
                                .bottom
                                .maybe_resolve(input.parent_size.height, |h, b| self.calc(h, b))
                                .map(|b| -b)
                        })
                        .unwrap_or(0.0),
                ]
            } else {
                [0.0; 2]
            };
            p.atom = Some(Atom {
                size: [out.size.width + margin.left + margin.right, height],
                baseline,
                offset: [margin.left + shift[0], margin.top + shift[1]],
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{front::Inputs, scene::Scene, time::Tick};

    fn load(body: &str) -> Scene {
        let source = format!(
            "export const composition={{width:320,height:240,fps:4,duration:2}}; export default function Scene(){{{body}}}"
        );
        Scene::load(
            "inline.motion.tsx",
            &|_| Some(source.as_bytes().to_vec()),
            &Inputs::default(),
            crate::testing::fonts(),
        )
        .unwrap()
    }

    #[test]
    fn inline_boxes_wrap_and_use_the_margin_box_baseline() {
        let mut scene = load(
            r#"return <Text style={{width:60,fontSize:20,lineHeight:"30px"}}>A<View key="box" style={{display:"inline-block",width:40,height:40,margin:"2px 3px",background:"red"}}/>B</Text>"#,
        );
        let frame = scene.frame(Tick::ZERO).unwrap();
        let lines = frame.layout.lines[0].as_ref().unwrap();
        assert_eq!(lines.atoms.len(), 1);
        let a = &lines.atoms[0];
        assert_eq!(a.rect.width, 46.0);
        assert_eq!(a.rect.height, 44.0);
        assert!((a.rect.y + 44.0 - lines.baselines[a.line as usize]).abs() < 0.001);
        assert_eq!(frame.layout.boxes[a.node as usize].border.width, 40.0);
        assert_eq!(&*lines.string, "AB");
        assert!(lines.boxes.len() >= 2);
    }

    #[test]
    fn inline_boxes_seek_without_retaining_sizes_or_pixels() {
        let body = r#"const {seconds}=useCurrentTime(); return <Text style={{width:180,fontSize:20,lineHeight:"32px"}}>Before <Span style={{verticalAlign:"super"}}><View style={{display:"inline-flex",width:40+seconds*20,gap:4,padding:3,background:"orange"}}><View style={{width:10,height:12,background:"red"}}/><View style={{width:10,height:20,background:"blue"}}/></View></Span> after words wrap.</Text>"#;
        let mut scene = load(body);
        for second in [0.0, 1.5, 0.25, 1.0, 0.0] {
            let at = Tick::from_seconds(second).unwrap();
            let frame = scene.frame(at).unwrap();
            let fresh = load(body).frame(at).unwrap();
            assert_eq!(frame.layout.boxes, fresh.layout.boxes);
            assert_eq!(frame.items, fresh.items);
            assert!(!frame.items.is_empty());
        }
    }

    #[test]
    fn boxes_keep_surrounding_spaces_percentages_and_relative_offsets() {
        let mut scene = load(
            r#"return <Text style={{width:200,height:100,fontSize:20,lineHeight:"30px"}}>A <View style={{display:"inline-block",width:"50%",height:"50%",position:"relative",left:3,top:4}}/> B</Text>"#,
        );
        let frame = scene.frame(Tick::ZERO).unwrap();
        let lines = frame.layout.lines[0].as_ref().unwrap();
        let atom = &lines.atoms[0];
        let rect = frame.layout.boxes[atom.node as usize].border;
        assert_eq!(&*lines.string, "A  B");
        assert_eq!([rect.width, rect.height], [100.0, 50.0]);
        assert!((rect.x - atom.rect.x - 3.0).abs() < 0.001);
        assert!((rect.y - atom.rect.y - 4.0).abs() < 0.001);
    }

    #[test]
    fn block_baseline_is_its_last_line_and_hidden_overflow_uses_its_bottom() {
        for overflow in ["visible", "hidden"] {
            let mut scene = load(&format!(
                r#"return <Text style={{{{width:200}}}}>A<View style={{{{display:"inline-block",width:80,padding:4,overflow:"{overflow}"}}}}><Text style={{{{fontSize:14,lineHeight:"20px"}}}}>two lines inside</Text></View>B</Text>"#
            ));
            let frame = scene.frame(Tick::ZERO).unwrap();
            let outer = frame.layout.lines[0].as_ref().unwrap();
            let inner = frame.layout.lines[2].as_ref().unwrap();
            assert_eq!(inner.boxes.len(), 2);
            let baseline = outer.baselines[0];
            let rect = frame.layout.boxes[1].border;
            let expected = if overflow == "hidden" {
                rect.y + rect.height
            } else {
                frame.layout.boxes[2].content.y + inner.baselines.last().unwrap()
            };
            assert!(
                (baseline - expected).abs() < 0.001,
                "{overflow}: {baseline} vs {expected}"
            );
        }
    }

    #[test]
    fn unsupported_block_and_out_of_flow_inputs_fail_explicitly() {
        for (style, message) in [
            ("width:10,height:10", "needs display: inline-block"),
            (
                "display:'inline-block',position:'absolute'",
                "out-of-flow boxes inside Text",
            ),
        ] {
            let mut scene = load(&format!(
                "return <Text><View style={{{{{style}}}}}/></Text>"
            ));
            assert!(scene.frame(Tick::ZERO).unwrap_err().contains(message));
        }
    }

    #[test]
    fn inline_combinations_keep_geometry_when_seeking() {
        for writing in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
            for direction in ["ltr", "rtl"] {
                for (index, align) in ["baseline", "middle", "top", "25%"].into_iter().enumerate() {
                    let display = ["inline-block", "inline-flex", "inline-grid"][index % 3];
                    let body = format!(
                        r#"const {{seconds}}=useCurrentTime(); return <Text style={{{{width:180,height:200,fontSize:"large",lineHeight:"30px",writingMode:"{writing}",direction:"{direction}"}}}}>AB <Span style={{{{fontSize:"smaller",fontVariantPosition:"super",verticalAlign:"{align}"}}}}>12Q?<View style={{{{display:"{display}",width:40+seconds*20,height:38,padding:3,margin:2,background:"orange",verticalAlign:"{align}",writingMode:"horizontal-tb"}}}}><Text style={{{{fontSize:"small",lineHeight:"20px"}}}}>OK</Text></View></Span> words wrap.</Text>"#,
                    );
                    let mut scene = load(&body);
                    for seconds in [1.5, 0.0, 0.5, 1.5] {
                        let at = Tick::from_seconds(seconds).unwrap();
                        let frame = scene.frame(at).unwrap();
                        let fresh = load(&body).frame(at).unwrap();
                        assert_eq!(
                            frame.layout.boxes, fresh.layout.boxes,
                            "{writing}/{direction}/{align}"
                        );
                        assert_eq!(frame.items, fresh.items, "{writing}/{direction}/{align}");
                        let lines = frame.layout.lines[0].as_ref().unwrap();
                        assert_eq!(lines.atoms.len(), 1);
                        let rect = frame.layout.boxes[lines.atoms[0].node as usize].border;
                        assert_eq!(
                            [rect.width, rect.height],
                            [40.0 + seconds as f32 * 20.0, 38.0]
                        );
                        for b in &frame.layout.boxes {
                            assert!(
                                [b.border.x, b.border.y, b.border.width, b.border.height]
                                    .iter()
                                    .all(|v| v.is_finite())
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn paragraphs_can_contain_only_boxes_and_vertical_boxes() {
        for writing in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
            let mut scene = load(&format!(
                r#"return <Text style={{{{width:140,height:160,writingMode:"{writing}"}}}}><View style={{{{display:"inline-block",width:30,height:40,background:"red"}}}}/><View style={{{{display:"inline-grid",width:40,height:30,background:"blue"}}}}/></Text>"#
            ));
            let frame = scene.frame(Tick::ZERO).unwrap();
            let lines = frame.layout.lines[0].as_ref().unwrap();
            assert_eq!(lines.atoms.len(), 2, "{writing}");
            assert_eq!(frame.layout.boxes[1].border.width, 30.0, "{writing}");
            assert_eq!(frame.layout.boxes[1].border.height, 40.0, "{writing}");
        }
    }
}
