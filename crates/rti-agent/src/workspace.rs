use std::path::{Path, PathBuf};
use std::process::Command;

/// An isolated git worktree on its own branch.
#[derive(Clone, Debug)]
pub struct Workspace {
    pub repo_root: PathBuf,
    pub path: PathBuf,
    pub branch: String,
    pub base_commit: String,
}

pub fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("git").args(args).current_dir(dir).output()?;
    if !out.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl Workspace {
    /// Create `.rti/worktrees/<id>` on branch `rti/<id>` from HEAD.
    pub fn create(repo_root: &Path, id: &str) -> anyhow::Result<Workspace> {
        let base_commit = git(repo_root, &["rev-parse", "HEAD"])?;
        let dir = repo_root.join(".rti").join("worktrees");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(id);
        let branch = format!("rti/{id}");
        if path.exists() {
            let _ = git(
                repo_root,
                &["worktree", "remove", "--force", path.to_str().unwrap()],
            );
        }
        let _ = git(repo_root, &["branch", "-D", &branch]);
        git(
            repo_root,
            &[
                "worktree",
                "add",
                "-b",
                &branch,
                path.to_str().unwrap(),
                "HEAD",
            ],
        )?;
        Ok(Workspace {
            repo_root: repo_root.to_path_buf(),
            path,
            branch,
            base_commit,
        })
    }

    pub fn diff_stat(&self) -> String {
        git(&self.path, &["diff", "--stat", "HEAD"]).unwrap_or_default()
    }

    pub fn diff(&self) -> String {
        git(&self.path, &["diff", "HEAD"]).unwrap_or_default()
    }

    pub fn has_changes(&self) -> bool {
        git(&self.path, &["status", "--porcelain"])
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// Commit everything in the worktree.
    pub fn commit(&self, message: &str) -> anyhow::Result<String> {
        git(&self.path, &["add", "-A"])?;
        git(
            &self.path,
            &[
                "-c",
                "user.name=RTI",
                "-c",
                "user.email=rti@localhost",
                "commit",
                "-q",
                "-m",
                message,
            ],
        )?;
        git(&self.path, &["rev-parse", "HEAD"])
    }

    /// Merge the task branch into the main checkout (fast-forward if
    /// possible). Refuses if the main checkout is dirty.
    pub fn merge_into_main(&self, message: &str) -> anyhow::Result<String> {
        let dirty = git(
            &self.repo_root,
            &["status", "--porcelain", "--untracked-files=no"],
        )?;
        if !dirty.is_empty() {
            anyhow::bail!(
                "main checkout has uncommitted changes; not merging {}",
                self.branch
            );
        }
        git(
            &self.repo_root,
            &[
                "-c",
                "user.name=RTI",
                "-c",
                "user.email=rti@localhost",
                "merge",
                "--no-edit",
                "-m",
                message,
                &self.branch,
            ],
        )?;
        git(&self.repo_root, &["rev-parse", "HEAD"])
    }

    pub fn remove(&self, delete_branch: bool) {
        let _ = git(
            &self.repo_root,
            &["worktree", "remove", "--force", self.path.to_str().unwrap()],
        );
        if delete_branch {
            let _ = git(&self.repo_root, &["branch", "-D", &self.branch]);
        }
    }
}
