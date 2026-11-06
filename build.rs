use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-env-changed=BUG_BUILD_ID");
    println!("cargo:rerun-if-env-changed=BUG_BUILD_COMMIT");
    for path in [
        "build.rs",
        "src",
        "Cargo.toml",
        "Cargo.lock",
        "native",
        "patches",
        "ladybird-revision",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    // Follow branch updates as well as detached HEADs (including Git worktrees).
    let mut refs = vec![
        "HEAD".to_owned(),
        "packed-refs".to_owned(),
        "index".to_owned(),
    ];
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
        refs.push(branch);
    }
    for reference in refs {
        if let Some(path) = git(&["rev-parse", "--git-path", &reference])
            && (std::path::Path::new(&path).exists() || reference.starts_with("refs/"))
        {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(files) = git(&["ls-files"]) {
        for path in files.lines() {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let commit = std::env::var("BUG_BUILD_COMMIT")
        .ok()
        .or_else(|| git(&["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unversioned".into());
    let version = std::env::var("BUG_BUILD_ID").unwrap_or_else(|_| {
        let short = &commit[..commit.len().min(12)];
        let dirty = git(&["status", "--porcelain", "--untracked-files=normal"])
            .is_some_and(|status| !status.is_empty());
        format!("snapshot-{short}{}", if dirty { "-dirty" } else { "" })
    });
    println!("cargo:rustc-env=BUG_VERSION={version}");
    println!("cargo:rustc-env=BUG_COMMIT={commit}");
}
