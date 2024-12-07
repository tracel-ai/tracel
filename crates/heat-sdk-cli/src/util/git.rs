use crate::print_err;
use std::process::Command;

pub struct GitRepo {
}

impl GitRepo {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {})
    }
    
    pub fn is_dirty(&self) -> anyhow::Result<bool> {
        let output = Command::new("git")
            .arg("diff")
            .arg("HEAD")
            .output()?;
        Ok(!output.stdout.is_empty())
    }

    pub fn if_not_dirty(self) -> anyhow::Result<Self> {
        if self.is_dirty()? {
            anyhow::bail!(
                "Repo is dirty. Please commit or stash your changes before running this command."
            );
        }

        Ok(self)
    }

    pub fn get_last_commit_hash(&self) -> anyhow::Result<String> {
        let last_commit_hash = Command::new("git")
            .arg("rev-parse")
            .arg("HEAD")
            .output()?;
        Ok(String::from_utf8(last_commit_hash.stdout)?.trim().to_string())
    }

    pub fn is_at_commit(&self, commit_hash: &str) -> anyhow::Result<bool> {
        let last_commit = self.get_last_commit_hash()?;

        Ok(last_commit == commit_hash)
    }

    pub fn checkout_commit(self, commit_hash: &str) -> anyhow::Result<CheckoutGuard> {
        // checkout commit, gix equivalent of:
        // git checkout <commit_hash>
        Command::new("git")
            .arg("checkout")
            .arg(commit_hash)
            .output()?;
        Ok(CheckoutGuard::new(Some(self)))
    }

    pub fn undo_checkout(&self) -> anyhow::Result<()> {
        // undo checkout, gix equivalent of:
        // git switch -
        Command::new("git")
            .arg("switch")
            .arg("-")
            .output()?;
        Ok(())
    }
}

pub struct CheckoutGuard {
    repo: Option<GitRepo>,
}

impl CheckoutGuard {
    pub(crate) fn new(repo: Option<GitRepo>) -> Self {
        Self { repo }
    }
}

impl Drop for CheckoutGuard {
    fn drop(&mut self) {
        if let Some(repo) = self.repo.take() {
            if let Err(e) = repo.undo_checkout() {
                print_err!("Failed to undo checkout: {}", e);
            }
        }
    }
}
