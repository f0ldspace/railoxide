use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    track_git_refs();

    let short_hash = git_short_hash().unwrap_or_else(|| "unknown".to_owned());
    println!("cargo::rustc-env=RAILOXIDE_GIT_SHORT_HASH={short_hash}");
}

fn track_git_refs() {
    let Some(manifest_dir) = env::var_os("CARGO_MANIFEST_DIR") else {
        return;
    };
    let git_dir = PathBuf::from(manifest_dir).join("../../.git");

    println!("cargo::rerun-if-changed={}", git_dir.join("HEAD").display());
    println!(
        "cargo::rerun-if-changed={}",
        git_dir.join("refs/heads").display()
    );
    println!(
        "cargo::rerun-if-changed={}",
        git_dir.join("packed-refs").display()
    );
}

fn git_short_hash() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let hash = String::from_utf8(output.stdout).ok()?;
    let hash = hash.trim();
    (!hash.is_empty()).then(|| hash.chars().take(7).collect())
}
