use crate::print_err;

pub struct GitRepo {
    repo: gix::Repository,
}

impl GitRepo {
    pub fn discover() -> anyhow::Result<Self> {
        let repo = gix::discover(".")?;
        Ok(Self { repo })
    }

    pub fn if_not_dirty(self) -> anyhow::Result<Self> {
        if self.repo.is_dirty()? {
            anyhow::bail!(
                "Repo is dirty. Please commit or stash your changes before running this command."
            );
        }

        Ok(self)
    }

    pub fn get_last_commit_hash(&self) -> anyhow::Result<String> {
        let last_commit = self.repo.head()?.peel_to_commit_in_place()?.id();
        Ok(last_commit.to_string())
    }

    pub fn is_at_commit(&self, commit_hash: &str) -> anyhow::Result<bool> {
        let last_commit = self.get_last_commit_hash()?;

        Ok(last_commit == commit_hash)
    }

    pub fn checkout_commit(self, _commit_hash: &str) -> anyhow::Result<CheckoutGuard> {
        // checkout commit, gix equivalent of:
        // git checkout <commit_hash>
        Ok(CheckoutGuard::new(Some(self)))
    }

    pub fn undo_checkout(&self) -> anyhow::Result<()> {
        // undo checkout, gix equivalent of:
        // git switch -
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
