use std::path::PathBuf;

/// `pcap` links `wpcap.lib`. Build it from the committed export list instead of needing the Npcap
/// SDK, and delay-load the DLL so the service still starts on machines without Npcap.
fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir"));
    let target = std::env::var("TARGET").expect("target");
    let def = manifest.join("lib").join("wpcap.def");
    println!("cargo:rerun-if-changed={}", def.display());

    let machine = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "X64",
        Ok("aarch64") => "ARM64",
        other => panic!("unsupported target arch {other:?}"),
    };
    let status = cc::windows_registry::find(&target, "lib.exe")
        .expect("lib.exe not found: install the Visual Studio C++ build tools")
        .arg("/NOLOGO")
        .arg(format!("/MACHINE:{machine}"))
        .arg(format!("/DEF:{}", def.display()))
        .arg(format!("/OUT:{}", out.join("wpcap.lib").display()))
        .status()
        .expect("running lib.exe");
    assert!(status.success(), "lib.exe failed: {status}");

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-arg=/DELAYLOAD:wpcap.dll");
    println!("cargo:rustc-link-lib=delayimp");
}
