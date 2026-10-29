fn main() {
    println!("cargo:rerun-if-changed=src/executor/skia/shader_error_handler.cc");
    if std::env::var_os("CARGO_FEATURE_NATIVE").is_some()
        && matches!(
            std::env::var("CARGO_CFG_TARGET_OS").as_deref(),
            Ok("macos" | "linux" | "windows")
        )
    {
        cc::Build::new()
            .cpp(true)
            .std("c++17")
            .file("src/executor/skia/shader_error_handler.cc")
            .compile("valle_shader_errors");
    }
}
