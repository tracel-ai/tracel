use anyhow::Context;

pub struct ProjectMetadata {
    pub pkg_name: String,
    pub bin_name: String,
    pub crate_name: String,
    pub pkg_dir: std::path::PathBuf,
    pub target_directory: std::path::PathBuf,
}

pub fn try_get_project_metadata(pkg_name: String, bin_name: String) -> anyhow::Result<ProjectMetadata> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .no_deps()
        .exec()
        .context("Failed to get project metadata")?;

    let pkg = metadata
        .packages
        .iter()
        .find(|pkg| pkg.name == pkg_name)
        .context("Failed to find package")?;

    let target_directory = metadata.target_directory.into_std_path_buf();

    let pkg_dir = pkg
        .manifest_path
        .parent()
        .ok_or(anyhow::anyhow!("Failed to get package directory"))?
        .to_path_buf()
        .into_std_path_buf();

    let crate_name = bin_name.replace("-", "_");
    let bin_name = bin_name.to_string();

    Ok(ProjectMetadata {
        pkg_name,
        bin_name,
        crate_name,
        pkg_dir,
        target_directory,
    })
}

pub fn try_relocate_cargo_build_bin_to_dir(
    target_dir: &std::path::PathBuf,
    bin_name: &str,
    new_dir: &std::path::PathBuf,
) -> anyhow::Result<std::path::PathBuf> {
    let bin_file_name = format!("{}{}", bin_name, std::env::consts::EXE_SUFFIX);
    let exe_dir = target_dir.join("debug").join(&bin_file_name);
    // todo find a better way to handle dep
    let dep_exe_dir = target_dir
        .join("debug")
        .join("deps")
        .join(&bin_file_name.replace("-", "_"));

    let new_dep_bin_name = format!("{}_dep_bootstrapped", bin_name);
    let new_bin_name = format!("{}_bootstrapped", bin_name);

    // I'm not entirely sure on the behavior of the file located in the deps directory
    // But one thing is for sure, it is being locked by the cargo run/build process at the time of running this code.
    if (dep_exe_dir.exists()) {
        let dep_bin_file = try_relocate_to_dir(&dep_exe_dir, new_dir, &new_dep_bin_name, false)
            .context("Failed to relocate main dep binary file")?;
        self_replace::self_delete_at(&dep_bin_file).context("Failed to self-delete dep file")?;
    }

    // leave copy in place for future cargo build cache
    let bootstrapped_program =
        try_move_to_dir_and_leave_copy_in_place(&exe_dir, new_dir, &new_bin_name)
            .context("Failed to relocate main binary file")?;

    Ok(bootstrapped_program)
}

fn is_run_with_cargo() -> bool {
    std::env::var("CARGO").is_ok()
}

fn find_nearest_manifest() -> Option<std::path::PathBuf> {
    let cwd = std::env::current_dir()
        .ok()
        .expect("Failed to get current directory");

    let mut path_ancestors = cwd.ancestors();

    for ancestor in path_ancestors {
        let manifest_path = ancestor.join("Cargo.toml");
        if manifest_path.exists() {
            return Some(manifest_path);
        }
    }

    None
}

fn cargo_locate_project() -> Option<std::path::PathBuf> {
    // cargo locate-project
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = std::process::Command::new(cargo)
        .arg("locate-project")
        .output()
        .ok()?;

    let output_str = String::from_utf8(output.stdout).expect("Failed to parse output");
    let parsed_output: serde_json::Value =
        serde_json::from_str(&output_str).expect("Failed to parse output");

    let manifest_path_str = parsed_output["root"].as_str()?;

    Some(manifest_path_str.into())
}

pub fn locate_project_dir() -> Option<std::path::PathBuf> {
    let manifest_path = {
        if is_run_with_cargo() {
            cargo_locate_project()
        } else {
            find_nearest_manifest()
        }
    }?;

    // println!("Found manifest at: {}", manifest_path.display());

    let manifest_dir = manifest_path.parent()?;

    Some(manifest_dir.to_path_buf())
}

pub fn try_relocate_to_dir(
    file: &std::path::Path,
    new_dir: &std::path::Path,
    new_name: &str,
    copy: bool,
) -> anyhow::Result<std::path::PathBuf> {
    anyhow::ensure!(file.exists(), "File does not exist");
    anyhow::ensure!(new_dir.exists(), "New directory does not exist");

    if file.starts_with(new_dir) {
        return Ok(file.to_path_buf());
    }

    let new_file_name = std::path::PathBuf::new()
        .with_file_name(new_name)
        .with_extension(file.extension().unwrap_or_default());
    let new_file = new_dir.join(new_file_name);

    if new_file.exists() {
        std::fs::remove_file(&new_file).context("Failed to remove existing file")?;
    }

    if copy {
        std::fs::copy(file, &new_file).context("Failed to copy file")?;
    } else {
        println!("Renaming {:?} to {:?}", file, new_file);
        std::fs::rename(file, &new_file).context("Failed to rename file")?;
    }

    Ok(new_file)
}

pub fn try_move_to_dir_and_leave_copy_in_place(
    file: &std::path::Path,
    new_dir: &std::path::Path,
    new_name: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let new_file = try_relocate_to_dir(file, new_dir, new_name, false)?;

    // leave a copy of the file in the original location
    std::fs::copy(&new_file, file).context("Failed to copy file")?;

    Ok(new_file)
}

/// Move the current executable to a temporary location and re-execute it.
fn self_replace() -> Result<(), Box<dyn std::error::Error>> {
    let current_exe = std::env::current_exe()?;
    let temp_name = format!("{}.tmp", current_exe.file_name().unwrap().to_string_lossy());
    let temp_exe = current_exe.with_file_name(temp_name);

    // Copy the current executable to a temporary file
    std::fs::copy(&current_exe, &temp_exe)?;

    self_replace::self_delete().expect("Failed to delete self");

    let process = cargo_util::ProcessBuilder::new(&temp_exe)
        .env("NO_SELF_REPLACE", "1")
        .args(&std::env::args_os().skip(1).collect::<Vec<_>>())
        .exec_replace();

    match process {
        Ok(_) => std::process::exit(0),
        Err(e) => {
            std::process::exit(e.downcast::<cargo_util::ProcessError>()?.code.unwrap_or(1));
        }
    }
}