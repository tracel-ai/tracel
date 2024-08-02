#![allow(dead_code)]

use std::fmt;

use serde::ser;

use super::interning::InternedString;

#[derive(PartialEq, Eq, Hash, Ord, PartialOrd, Clone, Debug, Copy)]
pub enum DepKind {
    Normal,
    Development,
    Build,
}

impl DepKind {
    pub fn kind_table(&self) -> &'static str {
        match self {
            DepKind::Normal => "dependencies",
            DepKind::Development => "dev-dependencies",
            DepKind::Build => "build-dependencies",
        }
    }
}

impl ser::Serialize for DepKind {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        match *self {
            DepKind::Normal => None,
            DepKind::Development => Some("dev"),
            DepKind::Build => Some("build"),
        }
        .serialize(s)
    }
}

/// FeatureValue represents the types of dependencies a feature can have.
#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum FeatureValue {
    /// A feature enabling another feature.
    Feature(InternedString),
    /// A feature enabling a dependency with `dep:dep_name` syntax.
    Dep { dep_name: InternedString },
    /// A feature enabling a feature on a dependency with `crate_name/feat_name` syntax.
    DepFeature {
        dep_name: InternedString,
        dep_feature: InternedString,
        /// If `true`, indicates the `?` syntax is used, which means this will
        /// not automatically enable the dependency unless the dependency is
        /// activated through some other means.
        weak: bool,
    },
}

impl FeatureValue {
    pub fn new(feature: InternedString) -> FeatureValue {
        match feature.split_once('/') {
            Some((dep, dep_feat)) => {
                let dep_name = dep.strip_suffix('?');
                FeatureValue::DepFeature {
                    dep_name: InternedString::new(dep_name.unwrap_or(dep)),
                    dep_feature: InternedString::new(dep_feat),
                    weak: dep_name.is_some(),
                }
            }
            None => {
                if let Some(dep_name) = feature.strip_prefix("dep:") {
                    FeatureValue::Dep {
                        dep_name: InternedString::new(dep_name),
                    }
                } else {
                    FeatureValue::Feature(feature)
                }
            }
        }
    }

    /// Returns the name of the dependency if and only if it was explicitly named with the `dep:` syntax.
    fn explicit_dep_name(&self) -> Option<InternedString> {
        match self {
            FeatureValue::Dep { dep_name, .. } => Some(*dep_name),
            _ => None,
        }
    }

    fn feature_or_dep_name(&self) -> InternedString {
        match self {
            FeatureValue::Feature(dep_name)
            | FeatureValue::Dep { dep_name, .. }
            | FeatureValue::DepFeature { dep_name, .. } => *dep_name,
        }
    }
}

impl fmt::Display for FeatureValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use self::FeatureValue::*;
        match self {
            Feature(feat) => write!(f, "{feat}"),
            Dep { dep_name } => write!(f, "dep:{dep_name}"),
            DepFeature {
                dep_name,
                dep_feature,
                weak,
            } => {
                let weak = if *weak { "?" } else { "" };
                write!(f, "{dep_name}{weak}/{dep_feature}")
            }
        }
    }
}
