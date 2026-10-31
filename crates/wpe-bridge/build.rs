fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let mut header = String::new();
    for (name, path) in [
        ("editing", "../../fixtures/editing.html"),
        ("second", "../../fixtures/second.html"),
    ] {
        let bytes = std::fs::read(path).unwrap();
        let escaped: String = bytes.iter().map(|b| format!("\\{:03o}", b)).collect();
        header.push_str(&format!(
            "static const char *fixture_{name} = \"{escaped}\";\n"
        ));
        println!("cargo:rerun-if-changed={path}");
    }
    std::fs::write(out.join("fixtures.h"), header).unwrap();
    let mut build = cc::Build::new();
    build.include(out);
    build
        .debug(true)
        .file("native/bridge.c")
        .flag("-std=c11")
        .warnings(true);
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
