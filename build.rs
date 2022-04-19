#[cfg(target_os = "linux")]
fn main() {
    println!("cargo:rerun-if-changed=src/client/transparent.c");

    cc::Build::new()
        .file("src/client/transparent.c")
        .compile("transparent_ext");
}

#[cfg(not(target_os = "linux"))]
fn main() {}
