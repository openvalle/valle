//! Linear DrawProgram section/token matching that emits the existing COPY/INSERT patch ABI.
//! Input sizes are bounded by the caller; unsupported layouts use its generic byte matcher.

pub(super) fn encode(base: &[u8], target: &[u8]) -> Option<Vec<u8>> {
    const TABLE: usize = 28 + 9 * 24;
    if base == target && !target.is_empty() {
        let mut p = TokenPatch::new(target.len());
        p.copy(0, target.len(), target);
        return Some(p.finish());
    }
    if base.len() < TABLE
        || target.len() < TABLE
        || &base[..8] != b"VLDRAW\0\0"
        || &target[..8] != b"VLDRAW\0\0"
    {
        return None;
    }
    let mut p = TokenPatch::new(target.len());
    p.insert(&target[..TABLE]);
    let mut expected_base = TABLE;
    let mut expected_target = TABLE;
    for section in 0..9 {
        let table = 28 + section * 24;
        let kind_base = u16::from_le_bytes(base[table..table + 2].try_into().unwrap());
        let kind = u16::from_le_bytes(target[table..table + 2].try_into().unwrap());
        let b0 = token_u64(base, table + 8)?;
        let t0 = token_u64(target, table + 8)?;
        let bn = token_u64(base, table + 16)?;
        let tn = token_u64(target, table + 16)?;
        if kind != kind_base
            || b0 != expected_base
            || t0 != expected_target
            || bn > base.len().saturating_sub(b0)
            || tn > target.len().saturating_sub(t0)
        {
            return None;
        }
        let b = &base[b0..b0 + bn];
        let t = &target[t0..t0 + tn];
        if b == t {
            p.copy(b0, tn, t);
        } else if (7..=9).contains(&kind)
            || (kind == 3
                && u32::from_le_bytes(target[8..12].try_into().unwrap())
                    == valle_draw::program::DRAW_PROGRAM_FORMAT_VERSION)
        {
            // Binary paths and instance columns have stable offsets within their own section,
            // even when preceding JSON numbers change their character length.
            if bn == tn {
                let mut i = 0;
                while i < tn {
                    let begin = i;
                    if b[i] == t[i] {
                        while i < tn && b[i] == t[i] {
                            i += 1;
                        }
                        p.copy(b0 + begin, i - begin, &t[begin..i]);
                    } else {
                        while i < tn && b[i] != t[i] {
                            i += 1;
                        }
                        p.insert(&t[begin..i]);
                    }
                }
            } else {
                p.insert(t);
            }
        } else {
            let mut bi = 0;
            let mut ti = 0;
            while bi < b.len() && ti < t.len() {
                // Static JSON regions are usually much longer than one token. Compare blocks
                // before invoking the token matcher; COPY still contains only byte-exact data,
                // and differing tokens retain the existing random-access patch contract.
                let begin = ti;
                let base_begin = bi;
                while bi + 32 <= b.len() && ti + 32 <= t.len() && b[bi..bi + 32] == t[ti..ti + 32] {
                    bi += 32;
                    ti += 32;
                }
                if ti != begin {
                    p.copy(b0 + base_begin, ti - begin, &t[begin..ti]);
                    if bi == b.len() || ti == t.len() {
                        break;
                    }
                }
                let be = token_end(b, bi);
                let te = token_end(t, ti);
                if b[bi..be] == t[ti..te] {
                    p.copy(b0 + bi, te - ti, &t[ti..te]);
                } else {
                    p.insert(&t[ti..te]);
                }
                bi = be;
                ti = te;
            }
            if ti < t.len() {
                p.insert(&t[ti..]);
            }
        }
        expected_base = b0 + bn;
        expected_target = t0 + tn;
    }
    if expected_target != target.len() || expected_base != base.len() {
        return None;
    }
    let out = p.finish();
    Some(if out.len() >= target.len() + 21 {
        token_literal(target)
    } else {
        out
    })
}

fn token_u64(bytes: &[u8], i: usize) -> Option<usize> {
    usize::try_from(u64::from_le_bytes(bytes.get(i..i + 8)?.try_into().ok()?)).ok()
}
fn token_end(bytes: &[u8], i: usize) -> usize {
    let mut end = i + 1;
    if bytes[i] == b'"' {
        while end < bytes.len() {
            match bytes[end] {
                b'\\' => end = (end + 2).min(bytes.len()),
                b'"' => return end + 1,
                _ => end += 1,
            }
        }
    } else if bytes[i] == b'-' || bytes[i].is_ascii_digit() {
        while end < bytes.len()
            && (bytes[end].is_ascii_digit()
                || matches!(bytes[end], b'.' | b'e' | b'E' | b'+' | b'-'))
        {
            end += 1;
        }
    } else if bytes[i].is_ascii_alphabetic() {
        while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
            end += 1;
        }
    }
    end
}

struct TokenPatch {
    bytes: Vec<u8>,
    literal: Vec<u8>,
    pending_copy: Option<(usize, usize, [u8; 16])>,
    instructions: u32,
}
impl TokenPatch {
    fn new(length: usize) -> Self {
        let mut bytes = Vec::with_capacity(length);
        bytes.extend_from_slice(b"VLDPAT\0\0");
        bytes.extend_from_slice(&(length as u32).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        Self {
            bytes,
            literal: Vec::new(),
            pending_copy: None,
            instructions: 0,
        }
    }
    fn copy(&mut self, offset: usize, length: usize, source: &[u8]) {
        if length == 0 {
            return;
        }
        if let Some((start, n, small)) = self.pending_copy.as_mut() {
            if *start + *n == offset {
                if *n < 16 {
                    let end = (*n + source.len()).min(16);
                    small[*n..end].copy_from_slice(&source[..end - *n]);
                }
                *n += length;
                return;
            }
        }
        self.flush_copy();
        let mut small = [0; 16];
        let end = source.len().min(16);
        small[..end].copy_from_slice(&source[..end]);
        self.pending_copy = Some((offset, length, small));
    }
    fn insert(&mut self, source: &[u8]) {
        self.flush_copy();
        self.literal.extend_from_slice(source);
    }
    fn flush_copy(&mut self) {
        if let Some((offset, length, small)) = self.pending_copy.take() {
            if length < 16 {
                self.literal.extend_from_slice(&small[..length]);
            } else {
                self.flush_literal();
                self.bytes.push(0);
                self.bytes.extend_from_slice(&(offset as u32).to_le_bytes());
                self.bytes.extend_from_slice(&(length as u32).to_le_bytes());
                self.instructions += 1;
            }
        }
    }
    fn flush_literal(&mut self) {
        if self.literal.is_empty() {
            return;
        }
        self.bytes.push(1);
        self.bytes
            .extend_from_slice(&(self.literal.len() as u32).to_le_bytes());
        self.bytes.extend_from_slice(&self.literal);
        self.literal.clear();
        self.instructions += 1;
    }
    fn finish(mut self) -> Vec<u8> {
        self.flush_copy();
        self.flush_literal();
        self.bytes[12..16].copy_from_slice(&self.instructions.to_le_bytes());
        self.bytes
    }
}
fn token_literal(target: &[u8]) -> Vec<u8> {
    let mut p = TokenPatch::new(target.len());
    p.insert(target);
    p.finish()
}

#[cfg(test)]
mod tests {
    use valle_draw::Rect;
    use valle_draw::program::{
        DrawProgram, DrawProgramBuilder, LinearColor, Node, Paint, PathData, PathNode, PathVerb,
    };

    fn path_program(changed: Option<usize>, extra_segment: bool) -> Vec<u8> {
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 1920.0, 1080.0));
        let paint = builder.push_paint(Paint::Solid(LinearColor::new(0.5, 0.2, 0.1, 1.0)));
        for i in 0..128 {
            let x = i as f64 * 4.0;
            let offset = if changed == Some(i) {
                std::f64::consts::PI
            } else {
                0.0
            };
            let mut verbs = vec![PathVerb::MoveTo, PathVerb::CubicTo, PathVerb::Close];
            let mut points = vec![
                [x + offset, 0.0],
                [x + 1.0, 2.0],
                [x + 3.0, 4.0],
                [x + 5.0, 8.0],
            ];
            if changed == Some(i) && extra_segment {
                verbs.insert(2, PathVerb::QuadTo);
                points.extend_from_slice(&[[x + 4.0, 9.0], [x + 2.0, 1.0]]);
            }
            let path = builder.push_path(PathData { verbs, points });
            let node = builder.push_node(Node::Path(PathNode {
                path,
                fill_rule: Default::default(),
                fill: Some(paint),
                stroke: None,
            }));
            builder.add_root(node);
        }
        builder.finish().unwrap().packed_bytes().unwrap()
    }

    #[test]
    fn binary_path_sections_preserve_f64_and_random_access_with_changed_row_lengths() {
        let base = path_program(None, false);
        for (changed, extra_segment) in [
            (Some(127), false),
            (Some(0), true),
            (Some(61), false),
            (None, false),
        ] {
            let target = path_program(changed, extra_segment);
            // Use the section matcher directly so this covers the binary PATHS branch rather
            // than succeeding via the generic byte matcher. Every seek binds to the same base.
            let patch = super::encode(&base, &target).unwrap();
            super::super::validate(&patch).unwrap();
            let output = super::super::apply(&base, &patch).unwrap();
            assert_eq!(output, target);
            assert_eq!(
                DrawProgram::from_packed(&output)
                    .unwrap()
                    .packed_bytes()
                    .unwrap(),
                target
            );
            if !extra_segment {
                assert!(patch.len() < target.len() / 4);
            }
        }
    }

    fn sections(json: &[u8], binary: &[u8]) -> Vec<u8> {
        let sections: Vec<(u16, &[u8])> = vec![
            (1, b"[1]"),
            (2, json),
            (7, binary),
            (8, b""),
            (9, b""),
            (3, json),
            (4, b"[]"),
            (5, b"{}"),
            (6, b"{}"),
        ];
        let mut out = vec![0; 244];
        out[..8].copy_from_slice(b"VLDRAW\0\0");
        let mut offset = 244;
        for (i, (kind, data)) in sections.into_iter().enumerate() {
            let e = 28 + i * 24;
            out[e..e + 2].copy_from_slice(&kind.to_le_bytes());
            out[e + 8..e + 16].copy_from_slice(&(offset as u64).to_le_bytes());
            out[e + 16..e + 24].copy_from_slice(&(data.len() as u64).to_le_bytes());
            out.extend_from_slice(data);
            offset += data.len();
        }
        out
    }
    #[test]
    fn variable_number_lengths_escaped_unicode_strings_and_shifted_binary_sections_roundtrip() {
        let base = sections(
            br#"[{"v":1,"s":"a\\b\"c","n":-1.3e-10,"x":null}]"#,
            &[0; 2048],
        );
        for text in [
            "[{\"v\":999999.01,\"s\":\"中文🙂\",\"n\":0,\"x\":true}]",
            "[]",
            "{\"x\":1}",
        ] {
            let mut binary = vec![0; 2048];
            binary[0] = 1;
            binary[2047] = 255;
            let target = sections(text.as_bytes(), &binary);
            let patch = super::super::encode(&base, &target).unwrap();
            super::super::validate(&patch).unwrap();
            assert_eq!(super::super::apply(&base, &patch).unwrap(), target);
        }
    }
    #[test]
    fn arbitrary_inputs_and_random_seeks_roundtrip() {
        let base = sections(b"[1.01,2.02,3.03]", &[1; 1024]);
        let mut seed = 41_u32;
        for i in 0..1000 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let json = format!("[{},-{}.12,\"{}\"]", seed, i, seed);
            let target = sections(json.as_bytes(), &seed.to_le_bytes());
            let patch = super::super::encode(&base, &target).unwrap();
            assert_eq!(super::super::apply(&base, &patch).unwrap(), target);
        }
        for b in [b"".as_slice(), b"bad"] {
            for t in [b"".as_slice(), b"short", b"longer unrelated target"] {
                assert_eq!(
                    super::super::apply(b, &super::super::encode(b, t).unwrap()).unwrap(),
                    t
                );
            }
        }
    }
}
