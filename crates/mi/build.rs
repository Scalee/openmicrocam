// The JPEG → NV12 shim (src/jpeg.c) against mozjpeg-sys's libjpeg-turbo headers.
fn main() {
    let inc = std::env::var("DEP_JPEG_INCLUDE").expect("mozjpeg-sys include path");
    let mut b = cc::Build::new();
    for p in std::env::split_paths(&inc) {
        b.include(p);
    }
    b.file("src/jpeg.c").opt_level(3).compile("microcam_jpeg");
    println!("cargo:rerun-if-changed=src/jpeg.c");
}
