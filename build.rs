//! Embeds the application icon (assets/cmdboard.ico) into cmdboard.exe.

fn main() {
    println!("cargo:rerun-if-changed=assets/cmdboard.rc");
    println!("cargo:rerun-if-changed=assets/cmdboard.ico");
    // The icon is cosmetic: without a resource compiler the build still succeeds.
    embed_resource::compile("assets/cmdboard.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("failed to embed the application icon");
}
