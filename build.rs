//! Makes the build stamp that `recon -V`, `--version` and the keymap overlay
//! show, and gives it to the crate: the `-V` line as the `RECON_VERSION` environment
//! variable, and the `--version` text as `$OUT_DIR/long_version.txt`.
//!
//! With no git, or outside a git checkout of this package, the stamp is the
//! release version and the build time. The build does not fail.

#[path = "src/stamp.rs"]
mod stamp;

use std::path::Path;
use std::process::Command;

fn main() {
    // A `rerun-if-changed` line turns off the Cargo default, which reruns the
    // script when any package file changes. So the package files must be
    // named here too, or an edit does not add `.dirty`. (An edit to a file
    // outside these paths adds `.dirty` only at the next `git add`, commit or
    // checkout.)
    for path in ["src", "build.rs", "Cargo.toml", "Cargo.lock"] {
        println!("cargo:rerun-if-changed={path}");
    }

    let version = std::env::var("CARGO_PKG_VERSION").expect("Cargo sets CARGO_PKG_VERSION");
    let built = jiff::Zoned::now()
        .strftime("%Y-%m-%dT%H:%M:%S%:z")
        .to_string();

    let checkout = Checkout::find();
    let git = checkout.as_ref().map(Checkout::facts);
    let short = stamp::short(&version, git.as_ref());
    let long = stamp::long(
        &short,
        checkout.as_ref().map(|c| c.toplevel.as_str()),
        &built,
    );

    println!("cargo:rustc-env=RECON_VERSION={short}");
    // A rustc-env value cannot contain a newline, so the long text goes in a
    // file that config.rs reads with `include_str!`.
    let out_dir = std::env::var("OUT_DIR").expect("Cargo sets OUT_DIR");
    std::fs::write(Path::new(&out_dir).join("long_version.txt"), long)
        .expect("OUT_DIR is writable");
}

/// The git checkout that contains this package.
struct Checkout {
    git_dir: String,
    common_dir: String,
    toplevel: String,
}

impl Checkout {
    /// `None` when there is no `git` program, no repository, or when the
    /// repository found does not track this package (for example, a source
    /// tarball unpacked inside some other repository).
    fn find() -> Option<Self> {
        git(&["ls-files", "--error-unmatch", "Cargo.toml"])?;
        // In a worktree, `.git` is a file: `HEAD` and the index are in the
        // worktree's own git directory, but the refs are in the common one.
        let out = git(&[
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
            "--show-toplevel",
        ])?;
        let mut lines = out.lines().map(str::to_string);
        Some(Self {
            git_dir: lines.next()?,
            common_dir: lines.next()?,
            toplevel: lines.next()?,
        })
    }

    fn facts(&self) -> stamp::Git {
        let head_ref = git(&["symbolic-ref", "-q", "HEAD"]);

        // A commit, a checkout, a reset or a `git add` changes one of these.
        // A new tag changes `refs/tags` or `packed-refs`. Each path is named
        // only if it exists: Cargo reruns the script on every build for a
        // path that does not exist, and after `git gc` a ref can be in
        // `packed-refs` only.
        watch(&format!("{}/HEAD", self.git_dir));
        watch(&format!("{}/index", self.git_dir));
        if let Some(head_ref) = &head_ref {
            watch(&format!("{}/{head_ref}", self.common_dir));
        }
        watch(&format!("{}/packed-refs", self.common_dir));
        watch(&format!("{}/refs/tags", self.common_dir));

        stamp::Git {
            since_tag: git(&["describe", "--tags", "--long", "--match", "v[0-9]*"])
                .and_then(|out| stamp::parse_describe(&out)),
            hash: git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_default(),
            // `--no-optional-locks` stops `git status` from writing the
            // index. Without it, a status that refreshes the index (after a
            // touch, or a save with no change) makes the index newer than
            // this build, and the next build reruns the script for nothing.
            // It also keeps the script from taking `index.lock` while you
            // run a git command of your own.
            dirty: git(&[
                "--no-optional-locks",
                "status",
                "--porcelain",
                "--untracked-files=no",
            ])
            .is_some_and(|out| !out.is_empty()),
            branch: head_ref.map(|r| r.trim_start_matches("refs/heads/").to_string()),
        }
    }
}

fn watch(path: &str) {
    if Path::new(path).exists() {
        println!("cargo:rerun-if-changed={path}");
    }
}

/// The trimmed output of a git command that succeeds, else `None`.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}
