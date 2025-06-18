use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;
use crate::print_err;

pub fn get_last_commit_hash() -> anyhow::Result<String> {
    let repo = gix::discover(".")?;
    let last_commit = repo.head()?.peel_to_commit_in_place()?.id();
    if repo.is_dirty()? {
        print_err!("Latest git commit: {}", last_commit);
        anyhow::bail!("Repo is dirty. Please commit or stash your changes before packaging.");
    }

    Ok(last_commit.to_string())
}

pub fn get_first_commit_hash() -> String {
    let repo = gix::discover(".")
        .ok()
        .expect("Failed to discover repository");

    let platform = repo
        .rev_walk([repo.head_id().expect("Failed to get HEAD id")])
        .first_parent_only()
        .sorting(Sorting::ByCommitTime(CommitTimeOrder::OldestFirst));
    let revs = platform.all().expect("Failed to get commits");

    let last_hash = revs
        .last()
        .expect("No commits found")
        .expect("Failed to get last commit")
        .id;

    last_hash.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_first_commit_hash() {
        let hash = get_first_commit_hash();

        let output = std::process::Command::new("git")
            .args(&["rev-list", "--parents", "HEAD"])
            .output()
            .expect("Failed to run git rev-list");

        let stdout = String::from_utf8_lossy(&output.stdout);

        if let Some(last_line) = stdout.lines().last() {
            let parts: Vec<&str> = last_line.split_whitespace().collect();
            if let Some(first_commit_hash) = parts.get(0) {
                assert_eq!(hash, *first_commit_hash);
            } else {
                panic!("No commit hash found in the last line of git rev-list output.");
            }
        } else {
            eprintln!("No commits found.");
        }
    }
}
