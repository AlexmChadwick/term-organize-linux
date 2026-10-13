fn main() {
    let mut build = cc::Build::new();
    build
        .include("vendor/lzfse/src")
        .flag_if_supported("-Wno-unused-function")
        .flag_if_supported("-Wno-unused-variable")
        .files([
            "vendor/lzfse/src/lzfse_decode.c",
            "vendor/lzfse/src/lzfse_decode_base.c",
            "vendor/lzfse/src/lzfse_encode.c",
            "vendor/lzfse/src/lzfse_encode_base.c",
            "vendor/lzfse/src/lzfse_fse.c",
            "vendor/lzfse/src/lzvn_decode_base.c",
            "vendor/lzfse/src/lzvn_encode_base.c",
        ]);
    build.compile("lzfse");
    println!("cargo:rerun-if-changed=vendor/lzfse/src");
}
