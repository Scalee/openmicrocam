//! Links the camera's MI libraries into the daemon. They have no NEEDED entries among themselves, so every one is
//! linked as needed-regardless (`--no-as-needed`) and found at run time through RPATH /config/lib (.cargo/config.toml).
//! The linker reads link-time copies of the camera's MI libraries from the directory `MICROCAM_LINK_LIBS` names: the
//! libraries of the camera's dump (`miservice/lib`), each with its file name set as its SONAME (`patchelf --set-soname`),
//! since the originals carry none and a linker would record their full path as NEEDED (docs/INSTALL.md «Build OpenMicroCam»). The value
//! comes from tools/hostconf.py, which reads the environment, the config file and the default, as every tool does.
use std::env;
use std::path::PathBuf;
use std::process::Command;

/// System libraries the MI libraries need beside libc (the vendor app needed GLIBC_2.29 `pow`, `log2`).
const SYSTEM_LIBS: &[&str] = &["m", "pthread", "dl", "rt"];

const LIBS: &[&str] = &[
    "mi_common", "cam_os_wrapper", "mi_sys", "mi_sensor", "mi_vif", "mi_isp", "cus3a", "ispalgo", "mi_scl",
    "mi_venc", "mi_disp", "mi_hdmi", "mi_rgn", "mi_gfx", "mi_vdec", "mi_jpd",
];

fn main() {
    stamp();
    let target = env::var("TARGET").unwrap();
    if !target.starts_with("arm") {
        return; // host build (cargo check / tests): nothing to link
    }
    println!("cargo:rerun-if-env-changed=MICROCAM_LINK_LIBS");
    println!("cargo:rerun-if-env-changed=MICROCAM_CONFIG");
    let libs = PathBuf::from(hostconf("MICROCAM_LINK_LIBS"));
    let libs = libs.canonicalize().unwrap_or_else(|_| panic!("MICROCAM_LINK_LIBS {} missing", libs.display()));
    println!("cargo:rerun-if-changed={}", libs.display());
    println!("cargo:rustc-link-arg=-L{}", libs.display());
    println!("cargo:rustc-link-arg=-Wl,--no-as-needed");
    for l in LIBS {
        println!("cargo:rustc-link-arg=-l{}", l);
    }
    // the MI libraries resolve their libc/libm/libpthread/libdl symbols through the executable's scope (no NEEDED
    // entries of their own): without -lm, libispalgo died on `pow` at the first 3A init (2026-10-24 trial)
    for l in SYSTEM_LIBS {
        println!("cargo:rustc-link-arg=-l{}", l);
    }
}

/// One setting of the tools (docs/TOOLS.md «Settings») as tools/hostconf.py has it.
fn hostconf(name: &str) -> String {
    let script = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../tools/hostconf.py");
    let out = Command::new("python3").arg(&script).output().unwrap_or_else(|e| panic!("python3 {}: {e}", script.display()));
    let text = String::from_utf8_lossy(&out.stdout);
    let prefix = format!("{name}=");
    text.lines().find_map(|l| l.strip_prefix(prefix.as_str()).map(str::to_string))
        .unwrap_or_else(|| panic!("{} gave no {name}: {}", script.display(), String::from_utf8_lossy(&out.stderr)))
}

/// The build's id for `microcam status` and `cam deploy`: `MICROCAM_BUILD` = «<commit>[-dirty] <UTC time>», dirty when the
/// tree has changes under what the build reads (the crates, the lock file, the cargo config).
fn stamp() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let run = |cmd: &str, args: &[&str]| {
        Command::new(cmd).args(args).current_dir(&root).output().ok().filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let commit = run("git", &["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "nogit".into());
    let changed = run("git", &["status", "--porcelain", "--untracked-files=no", "--", "crates", "Cargo.toml", "Cargo.lock", ".cargo"]);
    let dirty = if changed.is_some_and(|c| !c.is_empty()) { "-dirty" } else { "" };
    let time = run("date", &["-u", "+%Y-%m-%dT%H:%MZ"]).unwrap_or_default();
    println!("cargo:rustc-env=MICROCAM_BUILD={commit}{dirty} {time}");
    // the git directory is found by git: in a submodule `.git` is a file naming it
    let git = run("git", &["rev-parse", "--absolute-git-dir"]).map(PathBuf::from);
    for p in ["HEAD", "index", "logs/HEAD"] {
        if let Some(g) = &git {
            println!("cargo:rerun-if-changed={}", g.join(p).display());
        }
    }
    for p in ["crates", "Cargo.lock", ".cargo"] {
        println!("cargo:rerun-if-changed={}", root.join(p).display());
    }
}
