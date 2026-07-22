use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Deserializer, de};

pub type Entries<T> = BTreeMap<Identifier, T>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identifier(String);

impl Identifier {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdentifierError::Empty);
        }
        if value.contains("${") {
            return Err(IdentifierError::Interpolation);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Identifier {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for Identifier {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<String> for Identifier {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for Identifier {
    type Error = IdentifierError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for Identifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentifierError {
    Empty,
    Interpolation,
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("an identifier cannot be empty"),
            Self::Interpolation => {
                formatter.write_str("an identifier cannot contain interpolation")
            }
        }
    }
}

impl Error for IdentifierError {}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnvironmentName(String);

impl EnvironmentName {
    pub fn new(value: impl Into<String>) -> Result<Self, EnvironmentNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(EnvironmentNameError::Empty);
        }
        if value.contains('=') {
            return Err(EnvironmentNameError::EqualsSign);
        }
        if value.contains("${") {
            return Err(EnvironmentNameError::Interpolation);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for EnvironmentName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for EnvironmentName {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for EnvironmentName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<String> for EnvironmentName {
    type Error = EnvironmentNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for EnvironmentName {
    type Error = EnvironmentNameError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for EnvironmentName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentNameError {
    Empty,
    EqualsSign,
    Interpolation,
}

impl fmt::Display for EnvironmentNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("an environment name cannot be empty"),
            Self::EqualsSign => formatter.write_str("an environment name cannot contain `=`"),
            Self::Interpolation => {
                formatter.write_str("an environment name cannot contain interpolation")
            }
        }
    }
}

impl Error for EnvironmentNameError {}

macro_rules! raw_string_type {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
    };
}

raw_string_type!(LiteralString);
raw_string_type!(ScalarTemplate);
raw_string_type!(ProviderInstallArg);

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub targets: Entries<Target>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub platform: PlatformConstraint,
    #[serde(default)]
    pub providers: Entries<Provider>,
    #[serde(default)]
    pub packages: Entries<Package>,
    #[serde(default)]
    pub links: Entries<Link>,
    #[serde(default)]
    pub actions: Entries<Action>,
    #[serde(default)]
    pub profiles: Entries<Profile>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default)]
    pub providers: Entries<Provider>,
    #[serde(default)]
    pub packages: Entries<Package>,
    #[serde(default)]
    pub links: Entries<Link>,
    #[serde(default)]
    pub actions: Entries<Action>,
    #[serde(default)]
    pub profiles: Entries<Profile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformConstraint {
    pub os: OneOrMany<Identifier>,
    pub arch: Option<OneOrMany<Identifier>>,
    pub distro: Option<OneOrMany<Identifier>>,
    pub distro_family: Option<OneOrMany<Identifier>>,
    pub environment: Option<OneOrMany<Identifier>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Package {
    Provider(ProviderPackage),
    Manual(Box<ManualPackage>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum ProviderPackage {
    Batch(BatchProviderPackage),
    Single(SingleProviderPackage),
}

impl ProviderPackage {
    pub fn provider(&self) -> &Identifier {
        match self {
            Self::Single(package) => &package.provider,
            Self::Batch(package) => &package.provider,
        }
    }

    pub fn provider_args(&self) -> Option<&[LiteralString]> {
        match self {
            Self::Single(package) => package.provider_args.as_deref(),
            Self::Batch(package) => package.provider_args.as_deref(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SingleProviderPackage {
    pub provider: Identifier,
    pub provider_args: Option<Vec<LiteralString>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchProviderPackage {
    pub provider: Identifier,
    pub names: Vec<Identifier>,
    pub provider_args: Option<Vec<LiteralString>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualPackage {
    pub install: Action,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub probe: ExecAction,
    pub activate: Option<EnvironmentPatch>,
    pub ensure: Option<OneOrMany<ExecAction>>,
    pub install: ExecAction<ProviderInstallArg>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPatch {
    pub path_prepend: Option<OneOrMany<ScalarTemplate>>,
    pub path_append: Option<OneOrMany<ScalarTemplate>>,
    #[serde(default)]
    pub variables: BTreeMap<EnvironmentName, ScalarTemplate>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "A: Deserialize<'de>"))]
pub struct ExecAction<A = ScalarTemplate> {
    #[serde(rename = "type")]
    pub kind: Option<ExecActionType>,
    pub program: ScalarTemplate,
    #[serde(default)]
    pub args: Vec<A>,
    pub cwd: Option<ScalarTemplate>,
    pub env: Option<EnvironmentPatch>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecActionType {
    Exec,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub source: ScalarTemplate,
    pub target: ScalarTemplate,
    pub on_conflict: Option<LinkConflict>,
    pub on_missing_parent: Option<LinkMissingParent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkConflict {
    Error,
    ReplaceLink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkMissingParent {
    Create,
    Skip,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub check: Option<ExecAction>,
    pub exec: ExecAction,
}
