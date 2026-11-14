//! Generates the raw MI bindings for the camera's ARM ABI from our declarations in `mi.h`, and the Linux UAPI ones from the
//! ARM sysroot. Host builds of this crate are not meaningful: the struct layouts and the libraries are 32-bit ARM.
use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let header = manifest.join("mi.h");
    println!("cargo:rerun-if-changed={}", header.display());

    let target = env::var("TARGET").unwrap();
    let mut clang_args: Vec<String> = Vec::new();
    if target.starts_with("arm") {
        clang_args.push("--target=arm-linux-gnueabihf".into());
        clang_args.push("-isystem".into());
        clang_args.push("/usr/arm-linux-gnueabihf/include".into());
    }

    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy())
        .clang_args(&clang_args)
        .allowlist_function("MI_.*|CUS3A_.*")
        .allowlist_type("MI_.*")
        .allowlist_var("MI_.*|E_MI_.*")
        .default_enum_style(bindgen::EnumVariation::ModuleConsts)
        .derive_default(true)
        .derive_debug(true)
        .layout_tests(true)
        .use_core()
        .ctypes_prefix("::core::ffi")
        .generate()
        .expect("bindgen over mi.h failed");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs");
    bindings.write_to_file(&out).expect("writing bindings.rs");

    // Linux UAPI for the UVC gadget (V4L2 + g_uvc + UVC class constants), from the ARM sysroot. ioctl request
    // numbers are macros bindgen cannot expand; the `mi` crate computes them from these struct sizes.
    let uapi = manifest.join("uapi.h");
    println!("cargo:rerun-if-changed={}", uapi.display());
    let uapi_bindings = bindgen::Builder::default()
        .header(uapi.to_string_lossy())
        .clang_args(&clang_args)
        .allowlist_type("v4l2_.*|uvc_.*|usb_ctrlrequest")
        .allowlist_var("V4L2_.*|UVC_.*|USB_.*")
        .default_enum_style(bindgen::EnumVariation::ModuleConsts)
        .derive_default(true)
        .derive_debug(false)
        .layout_tests(false)
        .use_core()
        .ctypes_prefix("::core::ffi")
        .generate()
        .expect("bindgen over the UVC/V4L2 UAPI failed");
    uapi_bindings
        .write_to_file(PathBuf::from(env::var("OUT_DIR").unwrap()).join("uapi.rs"))
        .expect("writing uapi.rs");

    // Linking is the daemon's job (crates/microcamd/build.rs): this crate only declares the symbols.
}
