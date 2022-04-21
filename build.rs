#[cfg(target_os = "linux")]
fn main() {
    println!("cargo:rerun-if-changed=src/client/transparent/utils.c");

    cc::Build::new()
        .file("src/client/transparent/utils.c")
        .compile("transparent_utils");
}

#[cfg(not(target_os = "linux"))]
fn main() {}
