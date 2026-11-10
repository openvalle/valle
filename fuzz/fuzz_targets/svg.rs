#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &str| {
    let _ = valle_engine::svg::file::Document::parse(input, &mut Vec::new());
});
