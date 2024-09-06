#![allow(unused)]

use std::{fmt, str::FromStr};

use cargo_metadata::semver;
use serde::{Deserialize, Serialize};

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/core/features.rs#L186
///
/// The edition of the compiler ([RFC 2052])
///
/// The following sections will guide you how to add and stabilize an edition.
///
/// ## Adding a new edition
///
/// - Add the next edition to the enum.
/// - Update every match expression that now fails to compile.
/// - Update the [`FromStr`] impl.
/// - Update [`CLI_VALUES`] to include the new edition.
/// - Set [`LATEST_UNSTABLE`] to Some with the new edition.
/// - Add an unstable feature to the [`features!`] macro invocation below for the new edition.
/// - Gate on that new feature in [`toml`].
/// - Update the shell completion files.
/// - Update any failing tests (hopefully there are very few).
/// - Update unstable.md to add a new section for this new edition (see [this example]).
///
/// ## Stabilization instructions
///
/// - Set [`LATEST_UNSTABLE`] to None.
/// - Set [`LATEST_STABLE`] to the new version.
/// - Update [`is_stable`] to `true`.
/// - Set the editionNNNN feature to stable in the [`features!`] macro invocation below.
/// - Update any tests that are affected.
/// - Update the man page for the `--edition` flag.
/// - Update unstable.md to move the edition section to the bottom.
/// - Update the documentation:
///   - Update any features impacted by the edition.
///   - Update manifest.md#the-edition-field.
///   - Update the `--edition` flag (options-new.md).
///   - Rebuild man pages.
///
/// [RFC 2052]: https://rust-lang.github.io/rfcs/2052-epochs.html
/// [`FromStr`]: Edition::from_str
/// [`CLI_VALUES`]: Edition::CLI_VALUES
/// [`LATEST_UNSTABLE`]: Edition::LATEST_UNSTABLE
/// [`LATEST_STABLE`]: Edition::LATEST_STABLE
/// [this example]: https://github.com/rust-lang/cargo/blob/3ebb5f15a940810f250b68821149387af583a79e/src/doc/src/reference/unstable.md?plain=1#L1238-L1264
/// [`is_stable`]: Edition::is_stable
/// [`toml`]: crate::util::toml
/// [`features!`]: macro.features.html
#[derive(
    Default, Clone, Copy, Debug, Hash, PartialOrd, Ord, Eq, PartialEq, Serialize, Deserialize,
)]
pub enum Edition {
    /// The 2015 edition
    #[default]
    Edition2015,
    /// The 2018 edition
    Edition2018,
    /// The 2021 edition
    Edition2021,
    /// The 2024 edition
    Edition2024,
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/core/features.rs#L198
/// Some unused impls were removed.
impl Edition {
    /// The latest edition that is unstable.
    ///
    /// This is `None` if there is no next unstable edition.
    pub const LATEST_UNSTABLE: Option<Edition> = Some(Edition::Edition2024);
    /// The latest stable edition.
    pub const LATEST_STABLE: Edition = Edition::Edition2021;
    pub const ALL: &'static [Edition] = &[
        Self::Edition2015,
        Self::Edition2018,
        Self::Edition2021,
        Self::Edition2024,
    ];
    /// Possible values allowed for the `--edition` CLI flag.
    ///
    /// This requires a static value due to the way clap works, otherwise I
    /// would have built this dynamically.
    pub const CLI_VALUES: [&'static str; 4] = ["2015", "2018", "2021", "2024"];

    /// Returns the first version that a particular edition was released on
    /// stable.
    pub(crate) fn first_version(&self) -> Option<semver::Version> {
        use Edition::*;
        match self {
            Edition2015 => None,
            Edition2018 => Some(semver::Version::new(1, 31, 0)),
            Edition2021 => Some(semver::Version::new(1, 56, 0)),
            Edition2024 => None,
        }
    }

    /// Returns `true` if this edition is stable in this release.
    pub fn is_stable(&self) -> bool {
        use Edition::*;
        match self {
            Edition2015 => true,
            Edition2018 => true,
            Edition2021 => true,
            Edition2024 => false,
        }
    }

    /// Returns the previous edition from this edition.
    ///
    /// Returns `None` for 2015.
    pub fn previous(&self) -> Option<Edition> {
        use Edition::*;
        match self {
            Edition2015 => None,
            Edition2018 => Some(Edition2015),
            Edition2021 => Some(Edition2018),
            Edition2024 => Some(Edition2021),
        }
    }

    /// Returns the next edition from this edition, returning the last edition
    /// if this is already the last one.
    pub fn saturating_next(&self) -> Edition {
        use Edition::*;
        match self {
            Edition2015 => Edition2018,
            Edition2018 => Edition2021,
            Edition2021 => Edition2024,
            Edition2024 => Edition2024,
        }
    }

    /// Whether or not this edition supports the `rust_*_compatibility` lint.
    ///
    /// Ideally this would not be necessary, but editions may not have any
    /// lints, and thus `rustc` doesn't recognize it. Perhaps `rustc` could
    /// create an empty group instead?
    pub(crate) fn supports_compat_lint(&self) -> bool {
        use Edition::*;
        match self {
            Edition2015 => false,
            Edition2018 => true,
            Edition2021 => true,
            Edition2024 => true,
        }
    }

    /// Whether or not this edition supports the `rust_*_idioms` lint.
    ///
    /// Ideally this would not be necessary...
    pub(crate) fn supports_idiom_lint(&self) -> bool {
        use Edition::*;
        match self {
            Edition2015 => false,
            Edition2018 => true,
            Edition2021 => false,
            Edition2024 => false,
        }
    }
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/core/features.rs#L313
impl fmt::Display for Edition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Edition::Edition2015 => f.write_str("2015"),
            Edition::Edition2018 => f.write_str("2018"),
            Edition::Edition2021 => f.write_str("2021"),
            Edition::Edition2024 => f.write_str("2024"),
        }
    }
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/core/features.rs#L324
impl FromStr for Edition {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, anyhow::Error> {
        match s {
            "2015" => Ok(Edition::Edition2015),
            "2018" => Ok(Edition::Edition2018),
            "2021" => Ok(Edition::Edition2021),
            "2024" => Ok(Edition::Edition2024),
            s if s.parse().map_or(false, |y: u16| y > 2024 && y < 2050) => anyhow::bail!(
                "this version of Cargo is older than the `{}` edition, \
                 and only supports `2015`, `2018`, `2021`, and `2024` editions.",
                s
            ),
            s => anyhow::bail!(
                "supported edition values are `2015`, `2018`, `2021`, or `2024`, \
                 but `{}` is unknown",
                s
            ),
        }
    }
}
