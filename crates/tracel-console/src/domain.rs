use serde::{Deserialize, Serialize};
use tracel_client::console::{
    model::response::CreatedByUserResponse,
    project::{request::Visibility as ClientVisibility, response::ProjectResponse},
};

use crate::ConsoleError;

/// A console namespace and the kind of owner it represents.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Namespace {
    /// Namespace slug used in console routes.
    pub name: String,
    /// Kind of account that owns the namespace.
    pub kind: NamespaceKind,
}

impl Namespace {
    /// Creates a user namespace.
    pub fn user(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: NamespaceKind::User,
        }
    }

    /// Creates an organization namespace.
    pub fn organization(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: NamespaceKind::Organization,
        }
    }
}

impl AsRef<Namespace> for Namespace {
    fn as_ref(&self) -> &Namespace {
        self
    }
}

/// Kind of account that owns a console namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamespaceKind {
    /// A person's namespace.
    User,
    /// An organization's namespace.
    Organization,
}

/// The user associated with a live console session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// Console-local user identifier.
    pub id: i32,
    /// User's display and login name.
    pub username: String,
    /// User's email address.
    pub email: String,
    /// Namespace owned by the user.
    pub namespace: Namespace,
}

/// An organization visible to the current session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Organization {
    /// Organization display name.
    pub name: String,
    /// Organization namespace used in project routes.
    pub namespace: Namespace,
}

/// Visibility applied to a console project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// Only explicitly authorized users can see the project.
    #[default]
    Private,
    /// Anonymous callers can see the project.
    Public,
}

/// A project in a user or organization namespace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Project name within its namespace.
    pub name: String,
    /// Namespace that owns the project.
    pub namespace: Namespace,
    /// Project description.
    pub description: String,
    /// Username that created the project.
    pub created_by: String,
    /// Project access level.
    pub visibility: Visibility,
}

/// A compact user identity attached to model records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSummary {
    /// Console-local user identifier.
    pub id: i32,
    /// User's display and login name.
    pub username: String,
    /// Namespace owned by the user.
    pub namespace: String,
}

impl TryFrom<ProjectResponse> for Project {
    type Error = ConsoleError;

    fn try_from(value: ProjectResponse) -> Result<Self, Self::Error> {
        let kind = match value.namespace_type.as_str() {
            "user" => NamespaceKind::User,
            "organization" => NamespaceKind::Organization,
            other => {
                return Err(ConsoleError::InvalidResponse(format!(
                    "unknown namespace type `{other}`"
                )));
            }
        };

        Ok(Self {
            name: value.project_name,
            namespace: Namespace {
                name: value.namespace_name,
                kind,
            },
            description: value.description,
            created_by: value.created_by,
            visibility: match value.visibility {
                ClientVisibility::Private => Visibility::Private,
                ClientVisibility::Public => Visibility::Public,
            },
        })
    }
}

impl From<CreatedByUserResponse> for UserSummary {
    fn from(value: CreatedByUserResponse) -> Self {
        Self {
            id: value.id,
            username: value.username,
            namespace: value.namespace,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_project_fixture_defaults_to_private_visibility() {
        let wire: ProjectResponse = serde_json::from_str(
            r#"{
                "project_name": "vision",
                "namespace_name": "ada",
                "namespace_type": "user",
                "description": "",
                "created_by": "ada"
            }"#,
        )
        .unwrap();
        let project = Project::try_from(wire).unwrap();

        assert_eq!(project.visibility, Visibility::Private);
    }
}
