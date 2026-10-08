use std::path::PathBuf;

fn main() {
    let cpp_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../cpp");

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++20")
        .include(cpp_dir.join("include"))
        .file(cpp_dir.join("src/mempool.cpp"))
        .file(cpp_dir.join("src/terrain.cpp"))
        .file(cpp_dir.join("src/mesher.cpp"))
        .file(cpp_dir.join("src/stubs.cpp"))
        .flag_if_supported("-fno-exceptions")
        .flag_if_supported("-fno-rtti")
        .warnings(true);

    // 严格诊断全开、警告即错:GCC/Clang(含 NDK)一组,MSVC 一组;
    // flag_if_supported 探测保证跨编译器不误伤。
    for f in [
        "-Wextra",
        "-Wpedantic",
        "-Wshadow",
        "-Wconversion",
        "-Werror",
        "/permissive-",
        "/W4",
        "/WX",
    ] {
        build.flag_if_supported(f);
    }

    if std::env::var("PROFILE").as_deref() == Ok("debug") {
        build.define("MCV_DEBUG", "1");
    }

    build.compile("mcvcpp");
    println!("cargo:rerun-if-changed=../../cpp");
}
