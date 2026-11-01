fn main() {
    let mut build = cc::Build::new();
    build
        .debug(true)
        .file("native/bridge.c")
        .flag("-std=c11")
        .warnings(true);
    if std::env::var_os("CARGO_FEATURE_NATIVE_FAILURE_INJECTION").is_some() {
        build.define("BW_NATIVE_FAILURE_INJECTION", None);
    }
    for name in [
        "wpe-webkit-2.0",
        "wpe-platform-2.0",
        "egl",
        "glesv2",
        "wayland-egl",
        "xkbcommon",
        "libdrm",
    ] {
        let mut config = pkg_config::Config::new();
        if name.starts_with("wpe-") {
            config.atleast_version("2.54");
        }
        let library = config
            .probe(name)
            .unwrap_or_else(|e| panic!("Missing {name}: {e}. See README.md for setup."));
        build.includes(library.include_paths);
    }
    build.compile("gpui_wpe_bridge");
    println!("cargo:rerun-if-changed=native/bridge.c");
    println!("cargo:rerun-if-changed=native/bridge.h");
}
