//! Table and message rendering shared by every command.

use comfy_table::Table;
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use indicatif::HumanBytes;
use tracel::console::{Console, Namespace, Organization, Project, User, Visibility};
use tracel::models::{Model, Models};

pub fn current_user(console: &Console) -> anyhow::Result<()> {
    let Some(user) = console.me()? else {
        anyhow::bail!("the session is no longer valid; run `console login` again");
    };
    cliclack::log::success("Signed in")?;
    user_table(&user);
    Ok(())
}

pub fn user_table(user: &User) {
    let mut table = pretty_table();
    table.set_header(["User", "Email", "Namespace"]).add_row([
        user.username.as_str(),
        user.email.as_str(),
        user.namespace.name.as_str(),
    ]);
    println!("{table}");
}

pub fn organizations(orgs: &[Organization]) -> anyhow::Result<()> {
    if orgs.is_empty() {
        cliclack::log::info("No organizations")?;
        return Ok(());
    }

    let mut table = pretty_table();
    table.set_header(["Organization", "Namespace"]);
    for org in orgs {
        table.add_row([org.name.as_str(), org.namespace.name.as_str()]);
    }
    println!("{table}");
    Ok(())
}

pub fn projects(namespace: &Namespace, projects: &[Project]) -> anyhow::Result<()> {
    if projects.is_empty() {
        cliclack::log::warning(format!("No projects in {}", namespace.name))?;
        return Ok(());
    }

    let mut table = pretty_table();
    table.set_header(["Project", "Visibility", "Description"]);
    for project in projects {
        table.add_row([
            project.name.as_str(),
            visibility_label(project.visibility),
            project.description.as_str(),
        ]);
    }
    println!("{table}");
    Ok(())
}

pub fn project(project: &Project) {
    let mut table = pretty_table();
    table
        .set_header([
            "Project",
            "Namespace",
            "Visibility",
            "Created by",
            "Description",
        ])
        .add_row([
            project.name.as_str(),
            project.namespace.name.as_str(),
            visibility_label(project.visibility),
            project.created_by.as_str(),
            project.description.as_str(),
        ]);
    println!("{table}");
}

pub fn models(models: &Models, owner: &str, project: &str) -> anyhow::Result<()> {
    let listed = models.list()?;
    if listed.is_empty() {
        cliclack::log::warning(format!("No models in {owner}/{project}"))?;
        return Ok(());
    }

    let mut table = pretty_table();
    table.set_header(["Model", "Latest", "Version", "Size", "Publisher"]);

    for model in listed {
        let latest = model
            .latest_version
            .map(|version| format!("v{version}"))
            .unwrap_or_else(|| "-".to_string());
        let versions = models.list_versions(&model.name)?;

        if versions.is_empty() {
            table.add_row([model.name.as_str(), latest.as_str(), "-", "-", "-"]);
            continue;
        }

        for version in versions {
            table.add_row([
                model.name.clone(),
                latest.clone(),
                format!("v{}", version.version),
                HumanBytes(version.size_bytes).to_string(),
                version
                    .published_by
                    .unwrap_or_else(|| "unknown".to_string()),
            ]);
        }
    }

    cliclack::log::success(format!("Models in {owner}/{project}"))?;
    println!("{table}");
    Ok(())
}

pub fn model(models: &Models, model: &Model) -> anyhow::Result<()> {
    let mut summary = pretty_table();
    summary
        .set_header(["Model", "Description", "Versions", "Latest", "Published by"])
        .add_row([
            model.name.as_str(),
            model.description.as_deref().unwrap_or("-"),
            &model.version_count.to_string(),
            &model
                .latest_version
                .map(|version| format!("v{version}"))
                .unwrap_or_else(|| "-".to_string()),
            model.published_by.as_deref().unwrap_or("unknown"),
        ]);
    println!("{summary}");

    let versions = models.list_versions(&model.name)?;
    if versions.is_empty() {
        cliclack::log::info("No published versions")?;
        return Ok(());
    }

    let mut table = pretty_table();
    table.set_header(["Version", "Size", "Files", "Publisher"]);
    for version in versions {
        table.add_row([
            format!("v{}", version.version),
            HumanBytes(version.size_bytes).to_string(),
            version.manifest.files.len().to_string(),
            version
                .published_by
                .unwrap_or_else(|| "unknown".to_string()),
        ]);
    }
    println!("{table}");
    Ok(())
}

pub fn visibility_label(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Private => "private",
        Visibility::Public => "public",
    }
}

fn pretty_table() -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS);
    table
}
