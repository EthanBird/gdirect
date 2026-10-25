use std::{env, path::PathBuf, process::Command};
fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let prefix = root.join("target/libaria2");
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let static_ssl = env::var("OPENSSL_STATIC").as_deref() == Ok("1");
    assert!(
        prefix.join("lib/libaria2.a").exists(),
        "Run ./scripts/build-libaria2.sh first (pinned aria2 1.37.0)"
    );
    let mut bridge = cc::Build::new();
    bridge
        .cpp(true)
        .std("c++11")
        .file("native/bridge.cc")
        .include(prefix.join("include"));
    if os == "windows" {
        assert_eq!(
            env::var("CARGO_CFG_TARGET_ENV").unwrap(),
            "gnu",
            "Windows builds use the MinGW GNU toolchain"
        );
        bridge.cpp_link_stdlib_static(true);
        let output = Command::new("g++")
            .arg("-print-file-name=libstdc++.a")
            .output()
            .expect("MinGW g++ required");
        let library = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
        if let Some(parent) = library.parent() {
            println!("cargo:rustc-link-search=native={}", parent.display());
        }
    }
    bridge.compile("gdirect_bridge");
    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=aria2");
    let ssl = env::var("OPENSSL_PREFIX").ok().or_else(|| {
        (os == "macos").then(|| {
            let out = Command::new("brew")
                .args(["--prefix", "openssl@3"])
                .output()
                .expect("brew or OPENSSL_PREFIX required");
            assert!(out.status.success());
            String::from_utf8(out.stdout).unwrap().trim().into()
        })
    });
    if let Some(ssl) = &ssl {
        println!("cargo:rustc-link-search=native={ssl}/lib");
    }
    if static_ssl && os == "macos" {
        // Darwin's -l search can prefer a dylib even for Rust static native libraries.
        let ssl = ssl
            .as_ref()
            .expect("OPENSSL_PREFIX is required for macOS release builds");
        println!("cargo:rustc-link-arg={ssl}/lib/libssl.a");
        println!("cargo:rustc-link-arg={ssl}/lib/libcrypto.a");
    } else {
        let linkage = if static_ssl { "static=" } else { "" };
        println!("cargo:rustc-link-lib={linkage}ssl");
        println!("cargo:rustc-link-lib={linkage}crypto");
    }
    match os.as_str() {
        "macos" => println!("cargo:rustc-link-lib=framework=Security"),
        "windows" => {
            for lib in [
                "ws2_32", "wsock32", "gdi32", "winmm", "iphlpapi", "psapi", "crypt32", "bcrypt",
                "advapi32",
            ] {
                println!("cargo:rustc-link-lib={lib}");
            }
            println!("cargo:rustc-link-arg=-static");
        }
        "linux" if static_ssl => {
            println!("cargo:rustc-link-lib=dl");
            println!("cargo:rustc-link-lib=pthread");
        }
        _ => {}
    }
    for path in [
        "native/bridge.cc",
        "vendor/gdirect_route.h",
        "target/libaria2/lib/libaria2.a",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    for var in ["OPENSSL_PREFIX", "OPENSSL_STATIC"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
}
