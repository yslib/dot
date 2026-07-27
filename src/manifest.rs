use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::platform::PlatformInfo;
use crate::schema::{
    Action, Config, Entries, Link, Package, PlatformConstraint, Profile, Provider,
    SelectableEntries, SelectorIdentifier, Target,
};

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
        let (target_id, target) = select_target(config, requested_target)?;

        if !target.platform.matches(actual_platform) {
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
                        requested: profile.to_owned(),
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
}

struct IndexedProfile<'a> {
    path: String,
    chain: Vec<&'a Profile>,
}

fn index_profiles<'a>(
    target: &SelectorIdentifier,
    profiles: &'a SelectableEntries<Profile>,
) -> Result<BTreeMap<SelectorIdentifier, IndexedProfile<'a>>, ManifestError> {
    fn visit<'a>(
        target: &SelectorIdentifier,
        profiles: &'a SelectableEntries<Profile>,
        path: &mut Vec<String>,
        chain: &mut Vec<&'a Profile>,
        index: &mut BTreeMap<SelectorIdentifier, IndexedProfile<'a>>,
    ) -> Result<(), ManifestError> {
        for (profile_id, profile) in profiles {
            path.push(profile_id.to_string());
            chain.push(profile);
            let current_path = path.join("/");

            if let Some(existing) = index.get(profile_id.as_str()) {
                return Err(ManifestError::DuplicateProfile {
                    target: target.to_string(),
                    profile: profile_id.to_string(),
                    first_path: existing.path.clone(),
                    second_path: current_path,
                });
            }

            index.insert(
                profile_id.clone(),
                IndexedProfile {
                    path: current_path,
                    chain: chain.clone(),
                },
            );
            visit(target, &profile.profiles, path, chain, index)?;

            chain.pop();
            path.pop();
        }

        Ok(())
    }

    let mut index = BTreeMap::new();
    visit(
        target,
        profiles,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut index,
    )?;
    Ok(index)
}

fn select_target<'a>(
    config: &'a Config,
    requested: Option<&str>,
) -> Result<(&'a SelectorIdentifier, &'a Target), ManifestError> {
    let available = || {
        config
            .targets
            .keys()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };

    match requested {
        Some(target) => {
            config
                .targets
                .get_key_value(target)
                .ok_or_else(|| ManifestError::UnknownTarget {
                    requested: target.to_owned(),
                    available: available(),
                })
        }
        None => match config.targets.len() {
            0 => Err(ManifestError::NoTargets),
            1 => {
                let (id, target) = config
                    .targets
                    .first_key_value()
                    .expect("length checked above");
                Ok((id, target))
            }
            _ => Err(ManifestError::TargetRequired {
                available: available(),
            }),
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    NoTargets,
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
            Self::NoTargets => formatter.write_str("the configuration contains no targets"),
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
