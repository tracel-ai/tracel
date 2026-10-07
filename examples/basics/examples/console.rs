//! TRACEL_NAMESPACE=<owner> TRACEL_PROJECT=<project> cargo run -p basics --example console

use tracel::console::ProjectHandle;

fn main() -> anyhow::Result<()> {
    let project = ProjectHandle::from_env()?;

    match project.console().me()? {
        Some(user) => println!("signed in as {} ({})", user.username, user.namespace.name),
        None => println!("the session is no longer valid; sign in again"),
    }

    let models = project.models();

    for model in models.list()? {
        let latest = model
            .latest_version
            .map(|version| format!("v{version}"))
            .unwrap_or_else(|| "no versions yet".to_string());
        println!("\n{} — {latest}", model.name);

        for version in models.list_versions(&model.name)? {
            let number = version
                .version
                .map(|version| format!("v{version}"))
                .unwrap_or_else(|| version.id.to_string());
            println!(
                "  {number:<5} {:>12} bytes  {}",
                version.size_bytes,
                version
                    .published_by
                    .as_deref()
                    .unwrap_or("unknown publisher")
            );
        }
    }

    Ok(())
}
