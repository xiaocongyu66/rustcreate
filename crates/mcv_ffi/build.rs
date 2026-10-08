use std::path::PathBuf;

fn main() {
    let cpp_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../cpp");

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .include(cpp_dir.join("include"))
        .file(cpp_dir.join("src/mempool.cpp"))
        .file(cpp_dir.join("src/stubs.cpp"))
        .flag_if_supported("-fno-exceptions")
        .flag_if_supported("-fno-rtti")
        .warnings(true);

    if std::env::var("PROFILE").as_deref() == Ok("debug") {
        build.define("MCV_DEBUG", "1");
    }

    build.compile("mcvcpp");
    println!("cargo:rerun-if-changed=../../cpp");
}
