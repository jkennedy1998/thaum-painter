use std::process::Command;

/// Bakes the short git commit into the binary via `THAUM_BUILD_COMMIT` so the
/// boot log self-identifies which build a run came from ("which zip is this?").
fn main() {
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=THAUM_BUILD_COMMIT={commit}");
    println!("cargo:rerun-if-changed=build.rs");
    // A commit bump changes none of the package sources, so without watching
    // the git refs the cached build would keep the previous commit's stamp
    // (seen live: a shipped exe booted as an older commit than its zip).
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");
}
