//! Reading a project's models from the console.
//!
//! Everything here goes through the `tracel` facade, so this is also a check that the console
//! API is usable the way an application would reach it.
//!
//! TRACEL_NAMESPACE=<owner> TRACEL_PROJECT=<project> cargo run -p basics --example console

use tracel::console::{Console, Env, TracelCredentials};

fn main() -> anyhow::Result<()> {
    let console = Console::connect(env()?, &credentials()?)?;
    let (namespace, project) = project()?;

    match console.me()? {
        Some(user) => println!("signed in as {} ({})", user.username, user.namespace.name),
        None => println!("the session is no longer valid; sign in again"),
    }

    let models = console
        .project((namespace.as_str(), project.as_str()))
        .models();

    for model in models.list()? {
        let latest = model
            .latest_version
            .map(|version| format!("v{version}"))
            .unwrap_or_else(|| "no versions yet".to_string());
        println!("\n{} — {latest}", model.name);

        for version in models.list_versions(&model.name)? {
            println!(
                "  v{:<4} {:>12} bytes  {}",
                version.version,
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

/// Which console to talk to, from `TRACEL_ENV`. Development by default, so a local devstack
/// needs no configuration.
fn env() -> anyhow::Result<Env> {
    match std::env::var("TRACEL_ENV").as_deref() {
        Err(_) | Ok("development") => Ok(Env::Development),
        Ok("production") => Ok(Env::Production),
        Ok(other) => match other.strip_prefix("staging:").map(str::parse::<u8>) {
            Some(Ok(version)) => Ok(Env::Staging(version)),
            _ => anyhow::bail!(
                "unknown TRACEL_ENV={other:?}; expected `production`, `development`, or `staging:N`"
            ),
        },
    }
}

/// Credentials from `TRACEL_API_KEY` or `TRACEL_SESSION_TOKEN`.
fn credentials() -> anyhow::Result<TracelCredentials> {
    TracelCredentials::from_env().map_err(|_| {
        anyhow::anyhow!("set TRACEL_API_KEY or TRACEL_SESSION_TOKEN to reach the console")
    })
}

/// The project to read, from `TRACEL_NAMESPACE` and `TRACEL_PROJECT`.
fn project() -> anyhow::Result<(String, String)> {
    let namespace = std::env::var("TRACEL_NAMESPACE")
        .map_err(|_| anyhow::anyhow!("set TRACEL_NAMESPACE to the owner of the project"))?;
    let project = std::env::var("TRACEL_PROJECT")
        .map_err(|_| anyhow::anyhow!("set TRACEL_PROJECT to the project name"))?;
    Ok((namespace, project))
}
