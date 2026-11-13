use std::process::Command;

fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    println!("cargo:rerun-if-changed=data");
    println!("cargo:rerun-if-env-changed=GITTYHUB_VERSION");
    let status = Command::new("glib-compile-resources")
        .args([
            "--sourcedir=data",
            &format!("--target={out}/gittyhub.gresource"),
            "data/resources.gresource.xml",
        ])
        .status()
        .expect("glib-compile-resources not found (install the glib development tools)");
    assert!(status.success(), "glib-compile-resources failed");
}
