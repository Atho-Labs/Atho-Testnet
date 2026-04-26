use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let falcon_dir = manifest_dir.join("../../Falcon 512 ");

    let sources = [
        "codec.c", "common.c", "falcon.c", "fft.c", "fpr.c", "keygen.c", "rng.c", "shake.c",
        "sign.c", "vrfy.c",
    ];

    let mut build = cc::Build::new();
    build
        .include(&falcon_dir)
        .define("FALCON_FPNATIVE", Some("1"))
        .warnings(true)
        .extra_warnings(true)
        .flag_if_supported("-O3");

    for src in sources {
        let path = falcon_dir.join(src);
        println!("cargo:rerun-if-changed={}", path.display());
        build.file(path);
    }

    build.compile("atho_falcon");
}
