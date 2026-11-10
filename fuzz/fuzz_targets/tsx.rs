#![no_main]

use libfuzzer_sys::fuzz_target;

// Rewrite and validate source without executing it or reading imports from disk/network.
fuzz_target!(|input: &str| {
    let _ = valle_engine::front::files("scene.motion.tsx", &|path| {
        (path == "scene.motion.tsx").then(|| input.to_owned())
    });
});
