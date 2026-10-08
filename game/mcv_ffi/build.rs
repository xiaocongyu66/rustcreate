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

    // 严格诊断全开、警告即错:GCC/Clang(含 NDK)一组,MSVC 一组(warnings(true)
    // 已含 /W4);flag_if_supported 探测保证跨编译器不误伤。
    // /wd4530:MSVC STL 内部 function-try-block 提示——我们刻意不启用异常
    // (-fno-exceptions 的 MSVC 语义),该告警正是对这一选择的描述,定点豁免。
    for f in [
        "-Wextra",
        "-Wpedantic",
        "-Wshadow",
        "-Wconversion",
        "-Werror",
        "/permissive-",
        "/WX",
        "/wd4530",
    ] {
        build.flag_if_supported(f);
    }

    if std::env::var("PROFILE").as_deref() == Ok("debug") {
        build.define("MCV_DEBUG", "1");
    }

    build.compile("mcvcpp");
    println!("cargo:rerun-if-changed=../../cpp");
}
