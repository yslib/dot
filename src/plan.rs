use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::action::{CommandPreparationError, ExecutionEnvironment};
use crate::interpolation::{
    DotPaths, InterpolationError, PackageContext, ResolveContext, XdgPaths,
    resolve_environment_patch, resolve_exec_action, resolve_literal_string,
    resolve_provider_install_action,
};
use crate::manifest::EffectiveManifest;
use crate::platform::PlatformInfo;
use crate::schema::{
    Action, EnvironmentPatch, ExecAction, LinkConflict, LinkMissingParent, OneOrMany, Package,
    Provider,
};

type ProviderGroups = BTreeMap<(String, Vec<String>), Vec<String>>;

#[derive(Debug)]
pub struct ExecutionPlan {
    target: String,
    profile: Option<String>,
    platform: PlatformInfo,
    providers: Vec<PlannedProvider>,
    provider_batches: Vec<PlannedProviderBatch>,
    manual_packages: Vec<PlannedManualPackage>,
    actions: Vec<PlannedAction>,
    links: Vec<PlannedLink>,
}

impl ExecutionPlan {
    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    pub fn platform(&self) -> &PlatformInfo {
        &self.platform
    }

    pub fn providers(&self) -> &[PlannedProvider] {
        &self.providers
    }

    pub fn provider_batches(&self) -> &[PlannedProviderBatch] {
        &self.provider_batches
    }

    pub fn manual_packages(&self) -> &[PlannedManualPackage] {
        &self.manual_packages
    }

    pub fn actions(&self) -> &[PlannedAction] {
        &self.actions
    }

    pub fn links(&self) -> &[PlannedLink] {
        &self.links
    }
}

#[derive(Debug)]
pub struct PlannedProvider {
    id: String,
    activate: Option<EnvironmentPatch>,
    probe: ExecAction,
    ensure: Vec<ExecAction>,
}

impl PlannedProvider {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn activate(&self) -> Option<&EnvironmentPatch> {
        self.activate.as_ref()
    }

    pub fn probe(&self) -> &ExecAction {
        &self.probe
    }

    pub fn ensure(&self) -> &[ExecAction] {
        &self.ensure
    }
}

#[derive(Debug)]
pub struct PlannedProviderBatch {
    provider: String,
    provider_args: Vec<String>,
    packages: Vec<String>,
    install: ExecAction,
}

impl PlannedProviderBatch {
    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn provider_args(&self) -> &[String] {
        &self.provider_args
    }

    pub fn packages(&self) -> &[String] {
        &self.packages
    }

    pub fn install(&self) -> &ExecAction {
        &self.install
    }
}

#[derive(Debug)]
pub struct PlannedManualPackage {
    id: String,
    install: Action,
}

impl PlannedManualPackage {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn install(&self) -> &Action {
        &self.install
    }
}

#[derive(Debug)]
pub struct PlannedAction {
    id: String,
    action: Action,
}

impl PlannedAction {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn action(&self) -> &Action {
        &self.action
    }
}

#[derive(Debug)]
pub struct PlannedLink {
    id: String,
    source: PathBuf,
    target: PathBuf,
    on_conflict: LinkConflict,
    on_missing_parent: LinkMissingParent,
}

impl PlannedLink {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn target(&self) -> &Path {
        &self.target
    }

    pub fn on_conflict(&self) -> LinkConflict {
        self.on_conflict
    }

    pub fn on_missing_parent(&self) -> LinkMissingParent {
        self.on_missing_parent
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ExecutionPlanner<'a> {
    base_environment: &'a ExecutionEnvironment,
    dot_paths: DotPaths<'a>,
    xdg_paths: &'a XdgPaths,
    platform: &'a PlatformInfo,
}

impl<'a> ExecutionPlanner<'a> {
    pub const fn new(
        base_environment: &'a ExecutionEnvironment,
        dot_paths: DotPaths<'a>,
        xdg_paths: &'a XdgPaths,
        platform: &'a PlatformInfo,
    ) -> Self {
        Self {
            base_environment,
            dot_paths,
            xdg_paths,
            platform,
        }
    }

    pub fn plan(&self, manifest: &EffectiveManifest) -> Result<ExecutionPlan, PlanningError> {
        let groups = self.group_provider_packages(manifest)?;
        let (providers, provider_environments) = self.plan_providers(manifest)?;
        let provider_batches =
            self.plan_provider_batches(manifest, groups, &provider_environments)?;
        let manual_packages = self.plan_manual_packages(manifest)?;
        let actions = self.plan_actions(manifest)?;
        let links = self.plan_links(manifest)?;

        Ok(ExecutionPlan {
            target: manifest.target().to_owned(),
            profile: manifest.profile().map(str::to_owned),
            platform: self.platform.clone(),
            providers,
            provider_batches,
            manual_packages,
            actions,
            links,
        })
    }

    fn group_provider_packages(
        &self,
        manifest: &EffectiveManifest,
    ) -> Result<ProviderGroups, PlanningError> {
        let mut groups = BTreeMap::new();

        for (package_id, package) in manifest.packages() {
            let Package::Provider(package) = package else {
                continue;
            };
            let provider = package.provider().as_str();
            if !manifest.providers().contains_key(provider) {
                return Err(PlanningError::UnknownProvider {
                    package: package_id.to_string(),
                    provider: provider.to_owned(),
                });
            }

            let provider_args = package
                .provider_args()
                .unwrap_or_default()
                .iter()
                .map(resolve_literal_string)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|source| PlanningError::Interpolation {
                    context: format!("package `{package_id}` provider_args"),
                    source,
                })?;
            groups
                .entry((provider.to_owned(), provider_args))
                .or_insert_with(Vec::new)
                .push(package_id.to_string());
        }

        Ok(groups)
    }

    fn plan_providers(
        &self,
        manifest: &EffectiveManifest,
    ) -> Result<(Vec<PlannedProvider>, BTreeMap<String, ExecutionEnvironment>), PlanningError> {
        let mut plans = Vec::new();
        let mut environments = BTreeMap::new();

        for (provider_id, provider) in manifest.providers() {
            let mut environment = self.base_environment.clone();
            let activate = provider
                .activate
                .as_ref()
                .map(|activate| {
                    let context = ResolveContext::new(&environment, self.dot_paths, self.xdg_paths);
                    resolve_environment_patch(activate, &context)
                })
                .transpose()
                .map_err(|source| PlanningError::Interpolation {
                    context: format!("provider `{provider_id}` activate"),
                    source,
                })?;
            if let Some(activate) = &activate {
                environment.apply_patch(activate).map_err(|source| {
                    PlanningError::EnvironmentPatch {
                        provider: provider_id.to_string(),
                        source,
                    }
                })?;
            }

            let context = ResolveContext::new(&environment, self.dot_paths, self.xdg_paths);
            let probe = resolve_exec_action(&provider.probe, &context).map_err(|source| {
                PlanningError::Interpolation {
                    context: format!("provider `{provider_id}` probe"),
                    source,
                }
            })?;
            let ensure = resolve_ensure(provider, &context).map_err(|source| {
                PlanningError::Interpolation {
                    context: format!("provider `{provider_id}` ensure"),
                    source,
                }
            })?;

            environments.insert(provider_id.to_string(), environment);
            plans.push(PlannedProvider {
                id: provider_id.to_string(),
                activate,
                probe,
                ensure,
            });
        }

        Ok((plans, environments))
    }

    fn plan_provider_batches(
        &self,
        manifest: &EffectiveManifest,
        groups: ProviderGroups,
        environments: &BTreeMap<String, ExecutionEnvironment>,
    ) -> Result<Vec<PlannedProviderBatch>, PlanningError> {
        groups
            .into_iter()
            .map(|((provider_id, provider_args), packages)| {
                let provider = &manifest.providers()[provider_id.as_str()];
                let environment = &environments[provider_id.as_str()];
                if !provider_args.is_empty() {
                    let resolver_count = provider
                        .install
                        .args
                        .iter()
                        .filter(|argument| argument.as_str() == "${package:provider_args}")
                        .count();
                    if resolver_count != 1 {
                        return Err(PlanningError::ProviderArgsResolverCount {
                            provider: provider_id,
                            actual: resolver_count,
                        });
                    }
                }
                let package_context = PackageContext::new(&packages, &provider_args);
                let context = ResolveContext::new(environment, self.dot_paths, self.xdg_paths)
                    .with_package(package_context);
                let install = resolve_provider_install_action(&provider.install, &context)
                    .map_err(|source| PlanningError::Interpolation {
                        context: format!("provider `{provider_id}` install batch"),
                        source,
                    })?;

                Ok(PlannedProviderBatch {
                    provider: provider_id,
                    provider_args,
                    packages,
                    install,
                })
            })
            .collect()
    }

    fn plan_manual_packages(
        &self,
        manifest: &EffectiveManifest,
    ) -> Result<Vec<PlannedManualPackage>, PlanningError> {
        let context = ResolveContext::new(self.base_environment, self.dot_paths, self.xdg_paths);
        manifest
            .packages()
            .iter()
            .filter_map(|(package_id, package)| {
                let Package::Manual(package) = package else {
                    return None;
                };
                Some(
                    resolve_action(&package.install, &context)
                        .map(|install| PlannedManualPackage {
                            id: package_id.to_string(),
                            install,
                        })
                        .map_err(|source| PlanningError::Interpolation {
                            context: format!("manual package `{package_id}` install"),
                            source,
                        }),
                )
            })
            .collect()
    }

    fn plan_actions(
        &self,
        manifest: &EffectiveManifest,
    ) -> Result<Vec<PlannedAction>, PlanningError> {
        let context = ResolveContext::new(self.base_environment, self.dot_paths, self.xdg_paths);
        manifest
            .actions()
            .iter()
            .map(|(action_id, action)| {
                resolve_action(action, &context)
                    .map(|action| PlannedAction {
                        id: action_id.to_string(),
                        action,
                    })
                    .map_err(|source| PlanningError::Interpolation {
                        context: format!("action `{action_id}`"),
                        source,
                    })
            })
            .collect()
    }

    fn plan_links(&self, manifest: &EffectiveManifest) -> Result<Vec<PlannedLink>, PlanningError> {
        let context = ResolveContext::new(self.base_environment, self.dot_paths, self.xdg_paths);
        manifest
            .links()
            .iter()
            .map(|(link_id, link)| {
                let source = crate::interpolation::resolve_scalar_template(&link.source, &context)
                    .map_err(|source| PlanningError::Interpolation {
                        context: format!("link `{link_id}` source"),
                        source,
                    })?;
                let source = PathBuf::from(source);
                let source = if source.is_absolute() {
                    source
                } else {
                    self.dot_paths.config_dir().join(source)
                };
                let target = crate::interpolation::resolve_scalar_template(&link.target, &context)
                    .map(PathBuf::from)
                    .map_err(|source| PlanningError::Interpolation {
                        context: format!("link `{link_id}` target"),
                        source,
                    })?;
                if !target.is_absolute() {
                    return Err(PlanningError::RelativeLinkTarget {
                        link: link_id.to_string(),
                        target,
                    });
                }

                Ok(PlannedLink {
                    id: link_id.to_string(),
                    source,
                    target,
                    on_conflict: link.on_conflict.unwrap_or(LinkConflict::ReplaceLink),
                    on_missing_parent: link.on_missing_parent.unwrap_or(LinkMissingParent::Create),
                })
            })
            .collect()
    }
}

fn resolve_action(
    action: &Action,
    context: &ResolveContext<'_>,
) -> Result<Action, InterpolationError> {
    Ok(Action {
        check: action
            .check
            .as_ref()
            .map(|check| resolve_exec_action(check, context))
            .transpose()?,
        exec: resolve_exec_action(&action.exec, context)?,
    })
}

fn resolve_ensure(
    provider: &Provider,
    context: &ResolveContext<'_>,
) -> Result<Vec<ExecAction>, InterpolationError> {
    match &provider.ensure {
        None => Ok(Vec::new()),
        Some(OneOrMany::One(action)) => Ok(vec![resolve_exec_action(action, context)?]),
        Some(OneOrMany::Many(actions)) => actions
            .iter()
            .map(|action| resolve_exec_action(action, context))
            .collect(),
    }
}

#[derive(Debug)]
pub enum PlanningError {
    UnknownProvider {
        package: String,
        provider: String,
    },
    Interpolation {
        context: String,
        source: InterpolationError,
    },
    EnvironmentPatch {
        provider: String,
        source: CommandPreparationError,
    },
    ProviderArgsResolverCount {
        provider: String,
        actual: usize,
    },
    RelativeLinkTarget {
        link: String,
        target: PathBuf,
    },
}

impl fmt::Display for PlanningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownProvider { package, provider } => {
                write!(
                    formatter,
                    "package `{package}` references unknown provider `{provider}`"
                )
            }
            Self::Interpolation { context, source } => {
                write!(formatter, "failed to resolve {context}: {source}")
            }
            Self::EnvironmentPatch { provider, source } => {
                write!(
                    formatter,
                    "failed to apply provider `{provider}` activate: {source}"
                )
            }
            Self::ProviderArgsResolverCount { provider, actual } => write!(
                formatter,
                "provider `{provider}` install must contain exactly one `${{package:provider_args}}` argument for a nonempty provider_args batch; found {actual}"
            ),
            Self::RelativeLinkTarget { link, target } => write!(
                formatter,
                "link `{link}` target must be absolute after interpolation: `{}`",
                target.display()
            ),
        }
    }
}

impl Error for PlanningError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UnknownProvider { .. } => None,
            Self::Interpolation { source, .. } => Some(source),
            Self::EnvironmentPatch { source, .. } => Some(source),
            Self::ProviderArgsResolverCount { .. } => None,
            Self::RelativeLinkTarget { .. } => None,
        }
    }
}
