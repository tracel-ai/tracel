//! Resolves namespace/project/model/version arguments, prompting with a live-fetched
//! `cliclack::select` menu whenever one is left out.

use cliclack::select;
use indicatif::HumanBytes;
use tracel::console::{Console, Model, ModelVersion, Models, Namespace, Project};

use crate::cli::ProjectScope;
use crate::display::visibility_label;

/// Resolves a namespace argument, looking it up against the session's organizations so the
/// right namespace kind is used even when the caller only passed a bare name.
pub fn resolve_namespace(
    console: &Console,
    namespace: Option<String>,
) -> anyhow::Result<Namespace> {
    match namespace {
        Some(name) => {
            let is_organization = console
                .organizations()?
                .iter()
                .any(|org| org.namespace.name == name);
            Ok(if is_organization {
                Namespace::organization(name)
            } else {
                Namespace::user(name)
            })
        }
        None => pick_namespace(console),
    }
}

/// Resolves a namespace/project pair, prompting for whichever half is missing.
pub fn resolve_project(console: &Console, scope: ProjectScope) -> anyhow::Result<(String, String)> {
    let namespace = match scope.namespace {
        Some(name) => resolve_namespace(console, Some(name))?,
        None => pick_namespace(console)?,
    };
    let project = match scope.project {
        Some(project) => project,
        None => pick_project(console, &namespace)?.name,
    };
    Ok((namespace.name, project))
}

/// Resolves a model name, prompting from the project's model listing when it is missing.
pub fn resolve_model(models: &Models, model: Option<String>) -> anyhow::Result<Model> {
    match model {
        Some(name) => models.get(&name).map_err(Into::into),
        None => pick_model(models),
    }
}

/// Resolves a version number, prompting from the model's version listing when it is missing.
pub fn resolve_version(
    models: &Models,
    model: &str,
    version: Option<u32>,
) -> anyhow::Result<ModelVersion> {
    let versions = models.list_versions(model)?;
    if versions.is_empty() {
        anyhow::bail!("{model} has no published versions");
    }

    match version {
        Some(number) => versions
            .into_iter()
            .find(|version| version.version == number)
            .ok_or_else(|| anyhow::anyhow!("{model} has no version {number}")),
        None => pick_version(model, versions),
    }
}

fn pick_namespace(console: &Console) -> anyhow::Result<Namespace> {
    let me = console
        .me()?
        .ok_or_else(|| anyhow::anyhow!("run `console login` first"))?;

    let mut picker = select("Namespace").item(
        me.namespace.clone(),
        format!("{} (you)", me.username),
        "user",
    );
    for org in console.organizations()? {
        let namespace = org.namespace.clone();
        picker = picker.item(namespace, org.name, "organization");
    }
    picker.interact().map_err(Into::into)
}

fn pick_project(console: &Console, namespace: &Namespace) -> anyhow::Result<Project> {
    let projects = console.projects_of(namespace)?;
    if projects.is_empty() {
        anyhow::bail!("{} has no visible projects", namespace.name);
    }

    let mut picker = select(format!("Project in {}", namespace.name));
    for project in projects {
        let hint = visibility_label(project.visibility);
        let label = project.name.clone();
        picker = picker.item(project, label, hint);
    }
    picker.interact().map_err(Into::into)
}

fn pick_model(models: &Models) -> anyhow::Result<Model> {
    let listed = models.list()?;
    if listed.is_empty() {
        anyhow::bail!("no models in this project");
    }

    let mut picker = select("Model");
    for model in listed {
        let hint = model
            .latest_version
            .map(|version| format!("v{version}"))
            .unwrap_or_else(|| "no versions yet".to_string());
        let label = model.name.clone();
        picker = picker.item(model, label, hint);
    }
    picker.interact().map_err(Into::into)
}

fn pick_version(model: &str, versions: Vec<ModelVersion>) -> anyhow::Result<ModelVersion> {
    let mut picker = select(format!("Version of {model}"));
    for version in versions {
        let hint = format!(
            "{} — {}",
            HumanBytes(version.size_bytes),
            version.published_by.as_deref().unwrap_or("unknown")
        );
        let label = format!("v{}", version.version);
        picker = picker.item(version, label, hint);
    }
    picker.interact().map_err(Into::into)
}
