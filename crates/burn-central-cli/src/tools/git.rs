#![allow(dead_code)]

use crate::print_err;
use anyhow::Context;
use gix::Repository;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;
use std::path::Path;

pub fn get_last_commit_hash() -> anyhow::Result<String> {
    let repo = gix::discover(".")?;
    let last_commit = repo.head()?.peel_to_commit_in_place()?.id();

    Ok(last_commit.to_string())
}

pub fn is_repo_dirty() -> anyhow::Result<bool> {
    let repo = gix::discover(".")?;
    let is_dirty = repo.is_dirty()?;
    if is_dirty {
        print_err!(
            "The repository is dirty. Please commit or stash your changes before proceeding."
        );
    }
    Ok(is_dirty)
}

pub fn get_first_commit_hash() -> anyhow::Result<String> {
    let repo = gix::discover(".")?;

    let platform = repo
        .rev_walk([repo.head_id()?])
        .first_parent_only()
        .sorting(Sorting::ByCommitTime(CommitTimeOrder::OldestFirst));
    let revs = platform.all()?;

    let last_hash = revs.last().context("No commits found in the repository.")?;
    let last_hash = last_hash?.id();

    Ok(last_hash.to_string())
}

pub fn is_repo_initialized() -> bool {
    gix::discover(".").is_ok()
}

pub fn init_repo(dir: &Path) -> anyhow::Result<Repository> {
    if is_repo_initialized() {
        return Err(anyhow::anyhow!("Repository already initialized."));
    }

    let repo = gix::init(dir)?;
    Ok(repo)
}

pub fn write_gitignore() -> anyhow::Result<()> {
    let repo = gix::discover(".")?;
    let gitignore_content = include_str!("../../template.gitignore");
    let gitignore_path = repo.path().join(".gitignore");
    std::fs::write(gitignore_path, gitignore_content)
        .map_err(|e| anyhow::anyhow!("Failed to write .gitignore: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_first_commit_hash() {
        let hash = get_first_commit_hash().expect("Failed to get first commit hash");

        let output = std::process::Command::new("git")
            .args(["rev-list", "--parents", "HEAD"])
            .output()
            .expect("Failed to run git rev-list");

        let stdout = String::from_utf8_lossy(&output.stdout);

        if let Some(last_line) = stdout.lines().last() {
            let parts: Vec<&str> = last_line.split_whitespace().collect();
            if let Some(first_commit_hash) = parts.first() {
                assert_eq!(hash, *first_commit_hash);
            } else {
                panic!("No commit hash found in the last line of git rev-list output.");
            }
        } else {
            eprintln!("No commits found.");
        }
    }
}
