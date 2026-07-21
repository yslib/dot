use std::collections::BTreeMap;

use serde::Deserialize;

pub type Entries<T> = BTreeMap<String, T>;

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
    pub os: OneOrMany<String>,
    pub arch: Option<OneOrMany<String>>,
    pub distro: Option<OneOrMany<String>>,
    pub distro_family: Option<OneOrMany<String>>,
    pub environment: Option<OneOrMany<String>>,
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
#[serde(deny_unknown_fields)]
pub struct ProviderPackage {
    pub provider: String,
    pub provider_args: Option<Vec<String>>,
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
    pub install: ExecAction,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPatch {
    pub path_prepend: Option<OneOrMany<String>>,
    pub path_append: Option<OneOrMany<String>>,
    #[serde(default)]
    pub variables: Entries<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecAction {
    #[serde(rename = "type")]
    pub kind: Option<ExecActionType>,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<String>,
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
    pub source: String,
    pub target: String,
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
