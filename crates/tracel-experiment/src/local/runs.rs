use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use relative_path::{Component, RelativePath};

/// The `.gitignore` written in a runs directory this backend creates: everything in it stays out
/// of git and of code packages.
pub const RUNS_GITIGNORE: &str =
    "# Created by tracel: experiment runs recorded on this machine.\n*\n";

/// Creates `dir`, the directory runs are recorded under, with a [`RUNS_GITIGNORE`] in it, unless
/// it already exists.
pub fn create_runs_dir(dir: &Path) -> io::Result<()> {
    if let Some(parent) = dir.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    match fs::create_dir(dir) {
        Ok(()) => fs::write(dir.join(".gitignore"), RUNS_GITIGNORE),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

/// Creates the directory of the next run under `root`, numbered after the highest one there, and
/// returns its number and path.
pub fn create_local_run_dir(root: &Path) -> Result<(u64, PathBuf), std::io::Error> {
    let mut next_id = 1u64;

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        if let Some(id) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u64>().ok())
        {
            next_id = next_id.max(id + 1);
        }
    }

    loop {
        let run_root = root.join(next_id.to_string());
        match fs::create_dir(&run_root) {
            Ok(()) => return Ok((next_id, run_root)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                next_id += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

/// The directory `name` names directly inside `parent`, when `name`, read as a relative path, is
/// one plain component: `None` when it is empty, `.`, `..`, a path of several, or holds a `\`,
/// which separates paths on Windows only.
pub fn child_dir(parent: &Path, name: &str) -> Option<PathBuf> {
    if name.contains('\\') {
        return None;
    }
    let mut components = RelativePath::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(dir)), None) => Some(parent.join(dir)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::thread;

    use rstest::rstest;

    use super::*;
    use crate::local::test_support::run;

    #[test]
    fn a_runs_directory_it_creates_ignores_everything_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("project/runs");

        let _run = run(&runs, "mnist");

        let gitignore = fs::read_to_string(runs.join(".gitignore")).unwrap();
        assert!(
            gitignore.lines().any(|pattern| pattern == "*"),
            "{gitignore}"
        );
    }

    #[test]
    fn a_runs_directory_that_exists_is_left_as_it_is() {
        let dir = tempfile::tempdir().unwrap();

        let _run = run(dir.path(), "mnist");

        assert!(!dir.path().join(".gitignore").exists());
    }

    #[rstest]
    #[case::plain("mnist")]
    #[case::absolute("/mnist")]
    fn a_name_of_one_directory_names_that_directory_of_its_parent(#[case] name: &str) {
        let parent = Path::new("runs");

        assert_eq!(child_dir(parent, name), Some(parent.join("mnist")));
    }

    #[rstest]
    #[case::empty("")]
    #[case::current(".")]
    #[case::parent("..")]
    #[case::nested("sweep/lr-3")]
    #[case::windows_separator("..\\outside")]
    fn a_name_that_is_not_exactly_one_directory_is_refused(#[case] name: &str) {
        assert_eq!(child_dir(Path::new("runs"), name), None);
    }

    #[test]
    fn runs_are_numbered_from_one_under_the_experiment_name() {
        let dir = tempfile::tempdir().unwrap();

        let first = run(dir.path(), "mnist");
        let second = run(dir.path(), "mnist");
        let other = run(dir.path(), "other");

        assert_eq!(first.id().as_str(), "1");
        assert_eq!(second.id().as_str(), "2");
        assert_eq!(other.id().as_str(), "1");
        assert!(dir.path().join("mnist/1").is_dir());
        assert!(dir.path().join("mnist/2").is_dir());
        assert!(dir.path().join("other/1").is_dir());
    }

    #[test]
    fn numbering_continues_after_the_highest_existing_run() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("mnist/7")).unwrap();
        fs::create_dir_all(dir.path().join("mnist/notes")).unwrap();

        assert_eq!(run(dir.path(), "mnist").id().as_str(), "8");
    }

    #[test]
    fn a_file_named_like_the_next_run_is_numbered_past() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("mnist")).unwrap();
        fs::write(dir.path().join("mnist/1"), "").unwrap();

        assert_eq!(run(dir.path(), "mnist").id().as_str(), "2");
    }

    #[test]
    fn runs_started_at_once_are_numbered_from_one_without_gaps() {
        const RUNS: u64 = 8;
        let dir = tempfile::tempdir().unwrap();
        let barrier = Barrier::new(RUNS as usize);

        let mut ids: Vec<u64> = thread::scope(|scope| {
            let runs: Vec<_> = (0..RUNS)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        run(dir.path(), "mnist").id().parse().unwrap()
                    })
                })
                .collect();
            runs.into_iter().map(|run| run.join().unwrap()).collect()
        });
        ids.sort();

        assert_eq!(ids, (1..=RUNS).collect::<Vec<_>>());
    }
}
