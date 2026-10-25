use std::{env, path::PathBuf, process::Command};
fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let prefix = root.join("target/libaria2");
    assert!(
        prefix.join("lib/libaria2.a").exists(),
        "Run ./scripts/build-libaria2.sh first (pinned aria2 1.37.0)"
    );
    cc::Build::new()
        .cpp(true)
        .std("c++11")
        .file("native/bridge.cc")
        .include(prefix.join("include"))
        .compile("gdirect_bridge");
    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=aria2");
    if env::var("CARGO_CFG_TARGET_OS").unwrap() == "macos" {
        let ssl = env::var("OPENSSL_PREFIX").unwrap_or_else(|_| {
            let out = Command::new("brew")
                .args(["--prefix", "openssl@3"])
                .output()
                .expect("brew or OPENSSL_PREFIX required");
            assert!(out.status.success());
            String::from_utf8(out.stdout).unwrap().trim().into()
        });
        println!("cargo:rustc-link-search=native={ssl}/lib");
        println!("cargo:rustc-link-lib=framework=Security");
    }
    println!("cargo:rustc-link-lib=ssl");
    println!("cargo:rustc-link-lib=crypto");
    for path in [
        "native/bridge.cc",
        "vendor/gdirect_route.h",
        "target/libaria2/lib/libaria2.a",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=OPENSSL_PREFIX");
}
