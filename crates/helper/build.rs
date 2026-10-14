//! Embeds the icon and the application manifest into the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=assets/helper.rc");
    println!("cargo:rerun-if-changed=assets/faulk.ico");
    println!("cargo:rerun-if-changed=assets/faulk-helper.manifest");
    embed();
}

// embed-resource is a build dependency on Windows hosts only; building the Windows
// executable from another host is not supported yet.
#[cfg(windows)]
fn embed() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/helper.rc", embed_resource::NONE)
            .manifest_required()
            .expect("cannot compile the Windows resources (is rc.exe installed?)");
    }
}

#[cfg(not(windows))]
fn embed() {
    // Without the resources the tray icon cannot load and the helper stops at start.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        panic!("build the Windows helper on Windows: its icon resource cannot be embedded here");
    }
}
