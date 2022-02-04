fn main() {
    let webapp_out = format!("{}/webapp", std::env::var("OUT_DIR").unwrap());
    std::fs::create_dir_all(webapp_out.as_str()).unwrap();

    #[cfg(not(debug_assertions))]
    std::process::Command::new("yarn")
        .current_dir(std::env::current_dir().unwrap().join("web"))
        .env("BUILD_PATH", webapp_out.as_str())
        .arg("build")
        .output()
        .unwrap();
}
