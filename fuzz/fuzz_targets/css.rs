#![no_main]

use libfuzzer_sys::fuzz_target;
use valle_engine::style::Sheet;
use valle_engine::template::{Kind, Node, Prop, Template, V};

// The first line is a camelCase style property; the remaining text is its CSS value.
fuzz_target!(|input: &str| {
    let Some((property, value)) = input.split_once('\n') else {
        return;
    };
    let template = Template {
        nodes: vec![Node {
            kind: Kind::View,
            path: None,
            classes: Vec::new(),
            style: vec![(property.into(), Prop::Const(V::Str(value.into())))],
            children: Vec::new(),
            site: None,
        }],
        roots: vec![0],
        ..Default::default()
    };
    if let Ok(sheet) = Sheet::new(&template) {
        let _ = sheet.compute(&[], true, [320.0, 180.0], &[]);
    }
});
