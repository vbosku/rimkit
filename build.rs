fn main() {
    // Embeds assets/icon.ico in the exe (needs windres on PATH with the GNU toolchain; build.ps1 sets it).
    println!("cargo:rerun-if-changed=assets/rimkit.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    embed_resource::compile("assets/rimkit.rc", embed_resource::NONE).manifest_optional().unwrap();
}
