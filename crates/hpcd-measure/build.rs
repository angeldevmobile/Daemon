use std::env;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(hpcd_ebpf)");
    println!("cargo::rerun-if-env-changed=AYA_BUILD_SKIP");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    if matches!(env::var("AYA_BUILD_SKIP").as_deref(), Ok("1" | "true")) {
        println!("cargo::warning=AYA_BUILD_SKIP set: building without latency measurement");
        return;
    }
    let root = format!("{}/../hpcd-ebpf", env::var("CARGO_MANIFEST_DIR").unwrap());
    let package = aya_build::Package {
        name: "hpcd-ebpf",
        root_dir: &root,
        ..Default::default()
    };
    if let Err(e) = aya_build::build_ebpf([package], aya_build::Toolchain::default()) {
        panic!(
            "building the eBPF program failed: {e:#}\n\
             needs: rustup toolchain install nightly --component rust-src && cargo install bpf-linker\n\
             or set AYA_BUILD_SKIP=1 to build hpcctl without `measure`"
        );
    }
    println!("cargo::rustc-cfg=hpcd_ebpf");
}
