use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::platform::PlatformInfo;
use crate::schema::{
    Action, Config, Entries, Link, Package, PlatformConstraint, Profile, Provider,
    SelectableEntries, SelectorIdentifier, Target,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetEntry<'a> {
    pub id: &'a SelectorIdentifier,
    pub target: &'a Target,
    pub compatible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileEntry<'a> {
    pub id: &'a SelectorIdentifier,
    pub path: Vec<&'a SelectorIdentifier>,
}

impl ProfileEntry<'_> {
    pub fn depth(&self) -> usize {
        self.path.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestJobRef<'a> {
    Package(&'a SelectorIdentifier, &'a Package),
    Action(&'a SelectorIdentifier, &'a Action),
    Link(&'a SelectorIdentifier, &'a Link),
}

pub fn target_entries<'a>(
    config: &'a Config,
    actual_platform: &PlatformInfo,
) -> impl Iterator<Item = TargetEntry<'a>> + 'a {
    config
        .targets
        .iter()
        .map(|(id, target)| TargetEntry {
            id,
            target,
            compatible: target.platform.matches(actual_platform),
        })
        .collect::<Vec<_>>()
        .into_iter()
}

pub fn profile_entries<'a>(
    target_id: &SelectorIdentifier,
    target: &'a Target,
) -> Result<std::vec::IntoIter<ProfileEntry<'a>>, ManifestError> {
    Ok(collect_profile_catalog(target_id, &target.profiles)?
        .into_iter()
        .map(|profile| profile.entry)
        .collect::<Vec<_>>()
        .into_iter())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveManifest {
    target: SelectorIdentifier,
    profile: Option<SelectorIdentifier>,
    providers: Entries<Provider>,
    packages: SelectableEntries<Package>,
    links: SelectableEntries<Link>,
    actions: SelectableEntries<Action>,
}

impl EffectiveManifest {
    pub fn select(
        config: &Config,
        actual_platform: &PlatformInfo,
        requested_target: Option<&str>,
        requested_profile: Option<&str>,
    ) -> Result<Self, ManifestError> {
        let target = requested_target
            .map(SelectorIdentifier::try_from)
            .transpose()
            .map_err(|_| ManifestError::UnknownTarget {
                requested: requested_target
                    .expect("conversion only fails for a supplied target")
                    .to_owned(),
                available: available_targets(config),
            })?;

        let profile = match requested_profile
            .map(SelectorIdentifier::try_from)
            .transpose()
        {
            Ok(profile) => profile,
            Err(_) => {
                let selected =
                    Self::select_for_execution(config, actual_platform, target.as_ref(), None)?;
                let target_id = config
                    .targets
                    .get_key_value(selected.target())
                    .expect("the selected target came from this configuration")
                    .0;
                let profiles = index_profiles(target_id, &config.targets[target_id].profiles)?;
                return Err(ManifestError::UnknownProfile {
                    target: target_id.to_string(),
                    requested: requested_profile
                        .expect("conversion only fails for a supplied profile")
                        .to_owned(),
                    available: profiles.keys().map(ToString::to_string).collect(),
                });
            }
        };

        Self::select_for_execution(config, actual_platform, target.as_ref(), profile.as_ref())
    }

    pub fn select_for_execution(
        config: &Config,
        actual_platform: &PlatformInfo,
        requested_target: Option<&SelectorIdentifier>,
        requested_profile: Option<&SelectorIdentifier>,
    ) -> Result<Self, ManifestError> {
        Self::select_with_mode(
            config,
            actual_platform,
            requested_target,
            requested_profile,
            SelectionMode::Execution,
        )
    }

    pub fn select_for_inspection(
        config: &Config,
        actual_platform: &PlatformInfo,
        requested_target: Option<&SelectorIdentifier>,
        requested_profile: Option<&SelectorIdentifier>,
    ) -> Result<Self, ManifestError> {
        Self::select_with_mode(
            config,
            actual_platform,
            requested_target,
            requested_profile,
            SelectionMode::Inspection,
        )
    }

    fn select_with_mode(
        config: &Config,
        actual_platform: &PlatformInfo,
        requested_target: Option<&SelectorIdentifier>,
        requested_profile: Option<&SelectorIdentifier>,
        mode: SelectionMode,
    ) -> Result<Self, ManifestError> {
        let target_was_explicit = requested_target.is_some();
        let (target_id, target) = select_target(config, actual_platform, requested_target)?;

        if (!target_was_explicit || mode == SelectionMode::Execution)
            && !target.platform.matches(actual_platform)
        {
            return Err(ManifestError::IncompatiblePlatform {
                target: target_id.to_string(),
                expected: Box::new(target.platform.clone()),
                actual: Box::new(actual_platform.clone()),
            });
        }

        let profiles = index_profiles(target_id, &target.profiles)?;
        let selected_profile = requested_profile
            .map(|profile| {
                profiles
                    .get_key_value(profile)
                    .ok_or_else(|| ManifestError::UnknownProfile {
                        target: target_id.to_string(),
                        requested: profile.to_string(),
                        available: profiles.keys().map(ToString::to_string).collect(),
                    })
            })
            .transpose()?;

        let mut providers = target.providers.clone();
        let mut packages = target.packages.clone();
        let mut links = target.links.clone();
        let mut actions = target.actions.clone();

        if let Some((_, selected)) = selected_profile {
            for profile in &selected.chain {
                providers.extend(profile.providers.clone());
                packages.extend(profile.packages.clone());
                links.extend(profile.links.clone());
                actions.extend(profile.actions.clone());
            }
        }

        Ok(Self {
            target: target_id.clone(),
            profile: selected_profile.map(|(profile_id, _)| profile_id.clone()),
            providers,
            packages,
            links,
            actions,
        })
    }

    pub fn target(&self) -> &str {
        self.target.as_str()
    }

    pub fn profile(&self) -> Option<&str> {
        self.profile.as_ref().map(SelectorIdentifier::as_str)
    }

    pub fn providers(&self) -> &Entries<Provider> {
        &self.providers
    }

    pub fn packages(&self) -> &SelectableEntries<Package> {
        &self.packages
    }

    pub fn links(&self) -> &SelectableEntries<Link> {
        &self.links
    }

    pub fn actions(&self) -> &SelectableEntries<Action> {
        &self.actions
    }

    pub fn unresolved_jobs(&self) -> impl Iterator<Item = ManifestJobRef<'_>> {
        let provider_packages = self.packages.iter().filter_map(|(id, package)| {
            matches!(package, Package::Provider(_)).then_some(ManifestJobRef::Package(id, package))
        });
        let manual_packages = self.packages.iter().filter_map(|(id, package)| {
            matches!(package, Package::Manual(_)).then_some(ManifestJobRef::Package(id, package))
        });
        let actions = self
            .actions
            .iter()
            .map(|(id, action)| ManifestJobRef::Action(id, action));
        let links = self
            .links
            .iter()
            .map(|(id, link)| ManifestJobRef::Link(id, link));

        provider_packages
            .chain(manual_packages)
            .chain(actions)
            .chain(links)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionMode {
    Execution,
    Inspection,
}

struct IndexedProfile<'a> {
    chain: Vec<&'a Profile>,
}

struct CollectedProfile<'a> {
    entry: ProfileEntry<'a>,
    chain: Vec<&'a Profile>,
}

fn collect_profile_catalog<'a>(
    target: &SelectorIdentifier,
    profiles: &'a SelectableEntries<Profile>,
) -> Result<Vec<CollectedProfile<'a>>, ManifestError> {
    fn visit<'a>(
        target: &SelectorIdentifier,
        profiles: &'a SelectableEntries<Profile>,
        path: &mut Vec<&'a SelectorIdentifier>,
        chain: &mut Vec<&'a Profile>,
        seen: &mut BTreeMap<&'a SelectorIdentifier, String>,
        catalog: &mut Vec<CollectedProfile<'a>>,
    ) -> Result<(), ManifestError> {
        for (profile_id, profile) in profiles {
            path.push(profile_id);
            chain.push(profile);
            let current_path = path
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("/");

            if let Some(first_path) = seen.get(profile_id) {
                return Err(ManifestError::DuplicateProfile {
                    target: target.to_string(),
                    profile: profile_id.to_string(),
                    first_path: first_path.clone(),
                    second_path: current_path,
                });
            }

            seen.insert(profile_id, current_path);
            catalog.push(CollectedProfile {
                entry: ProfileEntry {
                    id: profile_id,
                    path: path.clone(),
                },
                chain: chain.clone(),
            });
            visit(target, &profile.profiles, path, chain, seen, catalog)?;

            chain.pop();
            path.pop();
        }

        Ok(())
    }

    let mut catalog = Vec::new();
    visit(
        target,
        profiles,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut BTreeMap::new(),
        &mut catalog,
    )?;
    Ok(catalog)
}

fn index_profiles<'a>(
    target: &SelectorIdentifier,
    profiles: &'a SelectableEntries<Profile>,
) -> Result<BTreeMap<SelectorIdentifier, IndexedProfile<'a>>, ManifestError> {
    let mut index = BTreeMap::new();
    for profile in collect_profile_catalog(target, profiles)? {
        index.insert(
            profile.entry.id.clone(),
            IndexedProfile {
                chain: profile.chain,
            },
        );
    }
    Ok(index)
}

fn available_targets(config: &Config) -> Vec<String> {
    config.targets.keys().map(ToString::to_string).collect()
}

fn select_target<'a>(
    config: &'a Config,
    actual_platform: &PlatformInfo,
    requested: Option<&SelectorIdentifier>,
) -> Result<(&'a SelectorIdentifier, &'a Target), ManifestError> {
    match requested {
        Some(target) => {
            config
                .targets
                .get_key_value(target)
                .ok_or_else(|| ManifestError::UnknownTarget {
                    requested: target.to_string(),
                    available: available_targets(config),
                })
        }
        None => {
            let mut compatible = config
                .targets
                .iter()
                .filter(|(_, target)| target.platform.matches(actual_platform));
            let first = compatible.next();
            let second = compatible.next();

            match (first, second) {
                (None, _) => Err(ManifestError::NoCompatibleTargets {
                    available: available_targets(config),
                }),
                (Some(target), None) => Ok(target),
                (Some(first), Some(second)) => {
                    let available = std::iter::once(first)
                        .chain(std::iter::once(second))
                        .chain(compatible)
                        .map(|(id, _)| id.to_string())
                        .collect();
                    Err(ManifestError::TargetRequired { available })
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    NoCompatibleTargets {
        available: Vec<String>,
    },
    TargetRequired {
        available: Vec<String>,
    },
    UnknownTarget {
        requested: String,
        available: Vec<String>,
    },
    IncompatiblePlatform {
        target: String,
        expected: Box<PlatformConstraint>,
        actual: Box<PlatformInfo>,
    },
    DuplicateProfile {
        target: String,
        profile: String,
        first_path: String,
        second_path: String,
    },
    UnknownProfile {
        target: String,
        requested: String,
        available: Vec<String>,
    },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCompatibleTargets { available } if available.is_empty() => {
                formatter.write_str("no configured targets are compatible with this platform")
            }
            Self::NoCompatibleTargets { available } => write!(
                formatter,
                "no configured targets are compatible with this platform; available targets: {}",
                available.join(", ")
            ),
            Self::TargetRequired { available } => write!(
                formatter,
                "a target is required; available targets: {}",
                available.join(", ")
            ),
            Self::UnknownTarget {
                requested,
                available,
            } => write!(
                formatter,
                "unknown target `{requested}`; available targets: {}",
                available.join(", ")
            ),
            Self::IncompatiblePlatform { target, .. } => {
                write!(
                    formatter,
                    "target `{target}` is incompatible with this platform"
                )
            }
            Self::DuplicateProfile {
                target,
                profile,
                first_path,
                second_path,
            } => write!(
                formatter,
                "profile `{profile}` is declared more than once in target `{target}`: `{first_path}` and `{second_path}`"
            ),
            Self::UnknownProfile {
                target,
                requested,
                available,
            } => write!(
                formatter,
                "unknown profile `{requested}` in target `{target}`; available profiles: {}",
                available.join(", ")
            ),
        }
    }
}

impl Error for ManifestError {}
