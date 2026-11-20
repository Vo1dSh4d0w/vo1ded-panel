use std::{env, path::PathBuf, process::Command};

// Styles are compiled with dart-sass (the same compiler AGS uses); the `grass`
// crate silently dropped declarations such as `background: transparent`.
fn main() {
    println!("cargo:rerun-if-changed=styles");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("style.css");
    let status = Command::new("sass")
        .args(["--no-source-map", "styles/style.scss"])
        .arg(&out)
        .status()
        .unwrap_or_else(|err| panic!("failed to run `sass` (dart-sass), is it installed? {err}"));
    assert!(status.success(), "sass failed to compile styles/style.scss");

    // Bundled icons; also emits `rerun-if-changed` for every listed file.
    glib_build_tools::compile_resources(
        &["resources"],
        "resources/resources.gresource.xml",
        "resources.gresource",
    );
}
