use crate::util::git::{CheckoutGuard, GitRepo};
use std::process::Command;
use crate::print_info;

pub struct CliGitRepo {}

impl CliGitRepo {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {})
    }
}

impl GitRepo for CliGitRepo {
    fn is_dirty(&self) -> anyhow::Result<bool> {
        let output = Command::new("git").arg("diff").arg("HEAD").output()?;
        Ok(!output.stdout.is_empty())
    }

    fn if_not_dirty(self) -> anyhow::Result<Self> {
        if self.is_dirty()? {
            anyhow::bail!(
                "Repo is dirty. Please commit or stash your changes before running this command."
            );
        }

        Ok(self)
    }

    fn get_last_commit_hash(&self) -> anyhow::Result<String> {
        let last_commit_hash = Command::new("git").arg("rev-parse").arg("HEAD").output()?;
        Ok(String::from_utf8(last_commit_hash.stdout)?
            .trim()
            .to_string())
    }

    fn is_at_commit(&self, commit_hash: &str) -> anyhow::Result<bool> {
        let last_commit = self.get_last_commit_hash()?;

        Ok(last_commit == commit_hash)
    }

    fn checkout_commit(self, commit_hash: &str) -> anyhow::Result<CheckoutGuard> {
        print_info!("Checking out commit: {}", commit_hash);
        Command::new("git")
            .arg("checkout")
            .arg(commit_hash)
            .output()?;
        Ok(CheckoutGuard::new(self))
    }

    fn undo_checkout(&self) -> anyhow::Result<()> {
        print_info!("Undoing checkout");
        Command::new("git").arg("switch").arg("-").output()?;
        Ok(())
    }
}
