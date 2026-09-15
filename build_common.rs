use std::process::Command;

fn main() {
    // Re-run script if Git HEAD or references change:
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/");

    // Execute `git describe` to get the latest tag or short commit hash:
    let git_tag = Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty=-modified"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| {
            let trimmed = s.trim();
            // Remove optional 'v' prefix from tags like "v1.2.0-rc.1":
            trimmed.strip_prefix('v').unwrap_or(trimmed).to_string()
        });

    // A version will be resolved from the following sources, in order of precedence:
    //
    // 1. Git tag/commit from `git describe`
    // 2. Fallback to Cargo.toml version
    let resolved_version = git_tag
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    // Inject CONTRACT_BUILD_VERSION into the contract's build context:
    println!("cargo:rustc-env=CONTRACT_BUILD_VERSION={}", resolved_version);
}