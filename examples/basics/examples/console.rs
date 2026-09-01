//! TRACEL_NAMESPACE=<owner> TRACEL_PROJECT=<project> cargo run -p basics --example console

use tracel::console::{Console, TracelCredentials};

fn main() -> anyhow::Result<()> {
    let console = Console::connect(&credentials()?)?;
    let (namespace, project) = project()?;

    match console.me()? {
        Some(user) => println!("signed in as {} ({})", user.username, user.namespace.name),
        None => println!("the session is no longer valid; sign in again"),
    }

    let models = console
        .project(namespace.as_str(), project.as_str())
        .models();

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

fn credentials() -> anyhow::Result<TracelCredentials> {
    TracelCredentials::from_env().map_err(|_| {
        anyhow::anyhow!("set TRACEL_API_KEY or TRACEL_SESSION_TOKEN to reach the console")
    })
}

fn project() -> anyhow::Result<(String, String)> {
    let namespace = std::env::var("TRACEL_NAMESPACE")
        .map_err(|_| anyhow::anyhow!("set TRACEL_NAMESPACE to the owner of the project"))?;
    let project = std::env::var("TRACEL_PROJECT")
        .map_err(|_| anyhow::anyhow!("set TRACEL_PROJECT to the project name"))?;
    Ok((namespace, project))
}
