mod cli;

use crate::print_err;
use crate::util::git::cli::CliGitRepo;

pub trait GitRepo {
    fn is_dirty(&self) -> anyhow::Result<bool>;
    fn if_not_dirty(self) -> anyhow::Result<Self>
    where
        Self: Sized;
    fn get_last_commit_hash(&self) -> anyhow::Result<String>;
    fn is_at_commit(&self, commit_hash: &str) -> anyhow::Result<bool>;
    fn checkout_commit(self, commit_hash: &str) -> anyhow::Result<CheckoutGuard>;
    fn undo_checkout(&self) -> anyhow::Result<()>;
}

pub type DefaultGitRepo = CliGitRepo;

pub struct CheckoutGuard {
    repo: Box<dyn GitRepo>,
}

impl CheckoutGuard {
    pub(crate) fn new(repo: impl GitRepo + 'static) -> Self {
        Self {
            repo: Box::new(repo) as Box<dyn GitRepo>,
        }
    }
}

impl Drop for CheckoutGuard {
    fn drop(&mut self) {
        if let Err(e) = self.repo.undo_checkout() {
            print_err!("Failed to undo checkout: {}", e);
        }
    }
}
