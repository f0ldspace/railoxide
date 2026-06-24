use std::env;
use std::fs;
use std::path::{Path, PathBuf};
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
    let git_path = PathBuf::from(manifest_dir).join("../../.git");

    if git_path.is_file() {
        println!("cargo::rerun-if-changed={}", git_path.display());
    }

    let Some(git_dir) = resolve_git_dir(&git_path) else {
        return;
    };

    let head_path = git_dir.join("HEAD");
    println!("cargo::rerun-if-changed={}", head_path.display());

    let Ok(head) = fs::read_to_string(&head_path) else {
        return;
    };

    let Some(ref_name) = active_ref(&head) else {
        return;
    };

    let common_dir_path = git_dir.join("commondir");
    let common_dir = fs::read_to_string(&common_dir_path).map_or_else(
        |_| git_dir.clone(),
        |common_dir| {
            println!("cargo::rerun-if-changed={}", common_dir_path.display());
            resolve_relative_path(&git_dir, common_dir.trim())
        },
    );

    let ref_path = common_dir.join(ref_name);
    println!("cargo::rerun-if-changed={}", ref_path.display());

    if !ref_path.exists() {
        println!(
            "cargo::rerun-if-changed={}",
            common_dir.join("packed-refs").display()
        );
    }
}

fn resolve_git_dir(git_path: &Path) -> Option<PathBuf> {
    if git_path.is_dir() {
        return Some(git_path.to_path_buf());
    }

    let git_file = fs::read_to_string(git_path).ok()?;
    let git_dir = git_file.trim().strip_prefix("gitdir:")?.trim();
    if git_dir.is_empty() {
        return None;
    }

    Some(resolve_relative_path(git_path.parent()?, git_dir))
}

fn resolve_relative_path(base: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn active_ref(head: &str) -> Option<&str> {
    head.trim()
        .strip_prefix("ref:")
        .map(str::trim)
        .filter(|ref_name| !ref_name.is_empty())
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
