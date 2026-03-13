use std::env;
use std::io;
use std::process::Command;
use winresource::WindowsResource;

fn main() -> io::Result<()> {
    slint_build::compile("ui/main.slint").expect("Slint build failed");

    let status = Command::new("cargo")
        .arg("build")
        .arg("--profile")
        .arg("release-lto")
        .arg("-p")
        .arg("launch")
        .status()
        .expect("Failed to execute secondary cargo build");

    if !status.success() {
        panic!("Secondary project build failed");
    }

    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        WindowsResource::new()
            .set_icon("assets/aoe2.ico")
            .set_manifest_file("assets/aoe-archive.manifest")
            .compile()?;
    }

    Ok(())
}
