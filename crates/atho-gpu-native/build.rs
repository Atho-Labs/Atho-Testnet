// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let workspace_native_dir = manifest_dir.join("../atho-node/native/gpu");
    let helper_src = workspace_native_dir.join("gpu_miner.cpp");
    let kernel_path = workspace_native_dir.join("sha3_384.cl");
    let cuda_kernel_path = workspace_native_dir.join("sha3_384.cu");

    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_GPU_NATIVE");
    println!("cargo:rerun-if-env-changed=CXX");
    println!("cargo:rerun-if-changed={}", helper_src.display());
    println!("cargo:rerun-if-changed={}", kernel_path.display());
    println!("cargo:rerun-if-changed={}", cuda_kernel_path.display());

    if env::var_os("CARGO_FEATURE_GPU_NATIVE").is_none() {
        return;
    }

    if !helper_src.exists() {
        panic!(
            "GPU miner helper source not found at {}; gpu-native feature is enabled but the native helper is missing",
            helper_src.display()
        );
    }

    let mut build = cc::Build::new();
    build.cpp(true);
    build.file(&helper_src);
    build.define("ATHO_GPU_BUILD_LIBRARY", None);
    build.warnings(false);
    build.opt_level(3);
    build.flag_if_supported("-std=c++17");
    build.flag_if_supported("/std:c++17");

    if let Err(err) = build.try_compile("atho_gpu_native") {
        panic!("failed to compile atho_gpu_native: {err}");
    }

    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=framework=OpenCL");
    } else {
        println!("cargo:rustc-link-lib=dylib=OpenCL");
    }
}
