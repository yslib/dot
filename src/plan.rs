use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::action::{CommandPreparationError, ExecutionEnvironment};
use crate::interpolation::{
    DotPaths, InterpolationError, PackageContext, ResolveContext, XdgPaths,
    promote_provider_install_args, provider_args_resolver_count, resolve_environment_patch,
    resolve_exec_action, resolve_literal_string, resolve_provider_install_action_with_args,
    resolve_string_expression,
};
use crate::job::{JobId, JobSelection, JobSelector};
use crate::manifest::EffectiveManifest;
use crate::platform::PlatformInfo;
use crate::schema::{
    Identifier, LinkConflict, LinkMissingParent, OneOrMany, Package, Provider, ProviderPackage,
    ResolvedAction, ResolvedEnvironmentPatch, ResolvedExecAction, SelectorIdentifier, SourceAction,
};

#[derive(Debug)]
pub struct ExecutionPlan {
    target: String,
    profile: Option<String>,
    platform: PlatformInfo,
    jobs: Vec<PlannedJob>,
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

    pub fn jobs(&self) -> &[PlannedJob] {
        &self.jobs
    }

    pub fn select(
        &self,
        selection: &JobSelection,
    ) -> Result<SelectedExecutionPlan<'_>, JobSelectionError> {
        let mut selected = BTreeSet::new();

        match selection {
            JobSelection::All => {
                selected.extend(self.jobs.iter().map(PlannedJob::id));
            }
            JobSelection::Only(selectors) => {
                for selector in selectors {
                    let job = self
                        .jobs
                        .iter()
                        .find(|job| job.matches_selector(selector))
                        .ok_or_else(|| JobSelectionError::Unknown(selector.clone()))?;

                    if let PlannedJob::Package(PlannedPackage::Provider(package)) = job {
                        let provider = package.provider_id();
                        if !self.jobs.iter().any(
                            |job| matches!(job, PlannedJob::Provider(job) if job.id() == provider.as_str()),
                        ) {
                            let package_id = match package {
                                PlannedProviderInstall::Single(package) => &package.id,
                                PlannedProviderInstall::Batch(package) => &package.id,
                            };
                            return Err(JobSelectionError::MissingProvider {
                                package: package_id.clone(),
                                provider: provider.clone(),
                            });
                        }
                        selected.insert(JobId::Provider(provider.clone()));
                    }

                    selected.insert(selector.job_id());
                }
            }
        }

        Ok(SelectedExecutionPlan {
            source: self,
            selected,
        })
    }

    pub fn providers(&self) -> impl Iterator<Item = &PlannedProvider> {
        self.jobs.iter().filter_map(|job| match job {
            PlannedJob::Provider(provider) => Some(provider),
            _ => None,
        })
    }

    pub fn provider_installs(&self) -> impl Iterator<Item = &PlannedProviderInstall> {
        self.jobs.iter().filter_map(|job| match job {
            PlannedJob::Package(PlannedPackage::Provider(package)) => Some(package),
            _ => None,
        })
    }

    pub fn manual_packages(&self) -> impl Iterator<Item = &PlannedManualPackage> {
        self.jobs.iter().filter_map(|job| match job {
            PlannedJob::Package(PlannedPackage::Manual(package)) => Some(package),
            _ => None,
        })
    }

    pub fn actions(&self) -> impl Iterator<Item = &PlannedAction> {
        self.jobs.iter().filter_map(|job| match job {
            PlannedJob::Action(action) => Some(action),
            _ => None,
        })
    }

    pub fn links(&self) -> impl Iterator<Item = &PlannedLink> {
        self.jobs.iter().filter_map(|job| match job {
            PlannedJob::Link(link) => Some(link),
            _ => None,
        })
    }
}

#[derive(Debug)]
pub struct SelectedExecutionPlan<'a> {
    source: &'a ExecutionPlan,
    selected: BTreeSet<JobId>,
}

impl SelectedExecutionPlan<'_> {
    pub const fn source(&self) -> &ExecutionPlan {
        self.source
    }

    pub fn jobs(&self) -> impl Iterator<Item = &PlannedJob> {
        let all_selected = self.selected.len() == self.source.jobs().len();
        self.source.jobs().iter().filter(move |job| {
            all_selected
                || self
                    .selected
                    .iter()
                    .any(|selected| job.matches_id(selected))
        })
    }
}

#[derive(Debug)]
pub enum PlannedJob {
    Provider(PlannedProvider),
    Package(PlannedPackage),
    Action(PlannedAction),
    Link(PlannedLink),
}

impl PlannedJob {
    pub fn id(&self) -> JobId {
        match self {
            Self::Provider(provider) => provider.job_id(),
            Self::Package(package) => package.job_id(),
            Self::Action(action) => action.job_id(),
            Self::Link(link) => link.job_id(),
        }
    }

    fn matches_id(&self, id: &JobId) -> bool {
        match (self, id) {
            (Self::Provider(provider), JobId::Provider(id)) => provider.id() == id.as_str(),
            (Self::Package(package), JobId::Package(id)) => package.id() == id.as_str(),
            (Self::Action(action), JobId::Action(id)) => action.id() == id.as_str(),
            (Self::Link(link), JobId::Link(id)) => link.id() == id.as_str(),
            _ => false,
        }
    }

    fn matches_selector(&self, selector: &JobSelector) -> bool {
        match (self, selector) {
            (Self::Package(package), JobSelector::Package(id)) => package.id() == id.as_str(),
            (Self::Action(action), JobSelector::Action(id)) => action.id() == id.as_str(),
            (Self::Link(link), JobSelector::Link(id)) => link.id() == id.as_str(),
            _ => false,
        }
    }
}

#[derive(Debug)]
pub enum PlannedPackage {
    Provider(PlannedProviderInstall),
    Manual(PlannedManualPackage),
}

impl PlannedPackage {
    fn id(&self) -> &str {
        match self {
            Self::Provider(package) => package.id(),
            Self::Manual(package) => package.id(),
        }
    }

    pub(crate) fn job_id(&self) -> JobId {
        match self {
            Self::Provider(package) => package.job_id(),
            Self::Manual(package) => package.job_id(),
        }
    }
}

#[derive(Debug)]
pub struct PlannedProvider {
    id: Identifier,
    activate: Option<ResolvedEnvironmentPatch>,
    probe: ResolvedExecAction,
    ensure: Vec<ResolvedExecAction>,
}

impl PlannedProvider {
    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    pub(crate) fn job_id(&self) -> JobId {
        JobId::Provider(self.id.clone())
    }

    pub fn activate(&self) -> Option<&ResolvedEnvironmentPatch> {
        self.activate.as_ref()
    }

    pub fn probe(&self) -> &ResolvedExecAction {
        &self.probe
    }

    pub fn ensure(&self) -> &[ResolvedExecAction] {
        &self.ensure
    }
}

#[derive(Debug)]
pub enum PlannedProviderInstall {
    Single(PlannedSingleProviderPackage),
    Batch(PlannedProviderPackageBatch),
}

impl PlannedProviderInstall {
    pub fn id(&self) -> &str {
        match self {
            Self::Single(package) => package.id.as_str(),
            Self::Batch(package) => package.id.as_str(),
        }
    }

    pub fn provider(&self) -> &str {
        match self {
            Self::Single(package) => package.provider.as_str(),
            Self::Batch(package) => package.provider.as_str(),
        }
    }

    pub(crate) fn job_id(&self) -> JobId {
        match self {
            Self::Single(package) => JobId::Package(package.id.clone()),
            Self::Batch(package) => JobId::Package(package.id.clone()),
        }
    }

    pub(crate) fn provider_id(&self) -> &Identifier {
        match self {
            Self::Single(package) => &package.provider,
            Self::Batch(package) => &package.provider,
        }
    }

    pub fn provider_args(&self) -> &[String] {
        match self {
            Self::Single(package) => &package.provider_args,
            Self::Batch(package) => &package.provider_args,
        }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        let (single, batch): (Option<&str>, &[String]) = match self {
            Self::Single(package) => (Some(package.id.as_str()), &[]),
            Self::Batch(package) => (None, &package.names),
        };
        single.into_iter().chain(batch.iter().map(String::as_str))
    }

    pub fn install(&self) -> &ResolvedExecAction {
        match self {
            Self::Single(package) => &package.install,
            Self::Batch(package) => &package.install,
        }
    }
}

#[derive(Debug)]
pub struct PlannedSingleProviderPackage {
    id: SelectorIdentifier,
    provider: Identifier,
    provider_args: Vec<String>,
    install: ResolvedExecAction,
}

#[derive(Debug)]
pub struct PlannedProviderPackageBatch {
    id: SelectorIdentifier,
    provider: Identifier,
    provider_args: Vec<String>,
    names: Vec<String>,
    install: ResolvedExecAction,
}

#[derive(Debug)]
pub struct PlannedManualPackage {
    id: SelectorIdentifier,
    install: ResolvedAction,
}

impl PlannedManualPackage {
    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    pub(crate) fn job_id(&self) -> JobId {
        JobId::Package(self.id.clone())
    }

    pub fn install(&self) -> &ResolvedAction {
        &self.install
    }
}

#[derive(Debug)]
pub struct PlannedAction {
    id: SelectorIdentifier,
    action: ResolvedAction,
}

impl PlannedAction {
    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    pub(crate) fn job_id(&self) -> JobId {
        JobId::Action(self.id.clone())
    }

    pub fn action(&self) -> &ResolvedAction {
        &self.action
    }
}

#[derive(Debug)]
pub struct PlannedLink {
    id: SelectorIdentifier,
    source: PathBuf,
    target: PathBuf,
    on_conflict: LinkConflict,
    on_missing_parent: LinkMissingParent,
}

impl PlannedLink {
    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    pub(crate) fn job_id(&self) -> JobId {
        JobId::Link(self.id.clone())
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
        let (providers, provider_environments) = self.plan_providers(manifest)?;
        let provider_installs = self.plan_provider_installs(manifest, &provider_environments)?;
        let manual_packages = self.plan_manual_packages(manifest)?;
        let actions = self.plan_actions(manifest)?;
        let links = self.plan_links(manifest)?;

        let mut jobs = Vec::new();
        jobs.extend(providers.into_iter().map(PlannedJob::Provider));
        jobs.extend(
            provider_installs
                .into_iter()
                .map(PlannedPackage::Provider)
                .map(PlannedJob::Package),
        );
        jobs.extend(
            manual_packages
                .into_iter()
                .map(PlannedPackage::Manual)
                .map(PlannedJob::Package),
        );
        jobs.extend(actions.into_iter().map(PlannedJob::Action));
        jobs.extend(links.into_iter().map(PlannedJob::Link));

        Ok(ExecutionPlan {
            target: manifest.target().to_owned(),
            profile: manifest.profile().map(str::to_owned),
            platform: self.platform.clone(),
            jobs,
        })
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
                id: provider_id.clone(),
                activate,
                probe,
                ensure,
            });
        }

        Ok((plans, environments))
    }

    fn plan_provider_installs(
        &self,
        manifest: &EffectiveManifest,
        environments: &BTreeMap<String, ExecutionEnvironment>,
    ) -> Result<Vec<PlannedProviderInstall>, PlanningError> {
        manifest
            .packages()
            .iter()
            .filter_map(|(package_id, package)| {
                let Package::Provider(package) = package else {
                    return None;
                };

                Some((|| {
                    let provider_id = package.provider();
                    let provider =
                        manifest
                            .providers()
                            .get(provider_id.as_str())
                            .ok_or_else(|| PlanningError::UnknownProvider {
                                package: package_id.to_string(),
                                provider: provider_id.to_string(),
                            })?;
                    let environment = &environments[provider_id.as_str()];
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
                    let names = match package {
                        ProviderPackage::Single(_) => vec![package_id.to_string()],
                        ProviderPackage::Batch(package) => {
                            if package.names.is_empty() {
                                return Err(PlanningError::EmptyPackageBatch {
                                    package: package_id.to_string(),
                                });
                            }
                            let mut seen = BTreeSet::new();
                            let mut names = Vec::with_capacity(package.names.len());
                            for name in &package.names {
                                if !seen.insert(name.as_str()) {
                                    return Err(PlanningError::DuplicatePackageBatchName {
                                        package: package_id.to_string(),
                                        name: name.to_string(),
                                    });
                                }
                                names.push(name.to_string());
                            }
                            names
                        }
                    };
                    let install_args = promote_provider_install_args(&provider.install.args)
                        .map_err(|source| PlanningError::Interpolation {
                            context: format!(
                                "provider `{provider_id}` install unit `{package_id}`"
                            ),
                            source,
                        })?;
                    if !provider_args.is_empty() {
                        let resolver_count = provider_args_resolver_count(&install_args);
                        if resolver_count != 1 {
                            return Err(PlanningError::ProviderArgsResolverCount {
                                provider: provider_id.to_string(),
                                actual: resolver_count,
                            });
                        }
                    }
                    let provider_args = provider_args
                        .into_iter()
                        .map(|argument| argument.value().to_owned())
                        .collect::<Vec<_>>();
                    let package_context = PackageContext::new(&names, &provider_args);
                    let context = ResolveContext::new(environment, self.dot_paths, self.xdg_paths)
                        .with_package(package_context);
                    let install = resolve_provider_install_action_with_args(
                        &provider.install,
                        &install_args,
                        &context,
                    )
                    .map_err(|source| PlanningError::Interpolation {
                        context: format!("provider `{provider_id}` install unit `{package_id}`"),
                        source,
                    })?;

                    Ok(match package {
                        ProviderPackage::Single(_) => {
                            PlannedProviderInstall::Single(PlannedSingleProviderPackage {
                                id: package_id.clone(),
                                provider: provider_id.clone(),
                                provider_args,
                                install,
                            })
                        }
                        ProviderPackage::Batch(_) => {
                            PlannedProviderInstall::Batch(PlannedProviderPackageBatch {
                                id: package_id.clone(),
                                provider: provider_id.clone(),
                                provider_args,
                                names,
                                install,
                            })
                        }
                    })
                })())
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
                            id: package_id.clone(),
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
                        id: action_id.clone(),
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
                let source =
                    resolve_string_expression(&link.source, &context).map_err(|source| {
                        PlanningError::Interpolation {
                            context: format!("link `{link_id}` source"),
                            source,
                        }
                    })?;
                let source = PathBuf::from(source.value());
                let source = if source.is_absolute() {
                    source
                } else {
                    self.dot_paths.config_dir().join(source)
                };
                let target = resolve_string_expression(&link.target, &context)
                    .map(|target| PathBuf::from(target.value()))
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
                    id: link_id.clone(),
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
    action: &SourceAction,
    context: &ResolveContext<'_>,
) -> Result<ResolvedAction, InterpolationError> {
    Ok(ResolvedAction {
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
) -> Result<Vec<ResolvedExecAction>, InterpolationError> {
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
    EmptyPackageBatch {
        package: String,
    },
    DuplicatePackageBatchName {
        package: String,
        name: String,
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
                "provider `{provider}` install must contain exactly one `${{package:provider_args}}` argument for an install unit with nonempty provider_args; found {actual}"
            ),
            Self::EmptyPackageBatch { package } => {
                write!(
                    formatter,
                    "package batch `{package}` must contain at least one name"
                )
            }
            Self::DuplicatePackageBatchName { package, name } => write!(
                formatter,
                "package batch `{package}` contains duplicate name `{name}`"
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
            Self::EmptyPackageBatch { .. } => None,
            Self::DuplicatePackageBatchName { .. } => None,
            Self::RelativeLinkTarget { .. } => None,
        }
    }
}

#[derive(Debug)]
pub enum JobSelectionError {
    Unknown(JobSelector),
    MissingProvider {
        package: SelectorIdentifier,
        provider: Identifier,
    },
}

impl fmt::Display for JobSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(JobSelector::Package(id)) => {
                write!(formatter, "unknown package job `{id}`")
            }
            Self::Unknown(JobSelector::Action(id)) => {
                write!(formatter, "unknown action job `{id}`")
            }
            Self::Unknown(JobSelector::Link(id)) => {
                write!(formatter, "unknown link job `{id}`")
            }
            Self::MissingProvider { package, provider } => write!(
                formatter,
                "package job `{package}` references missing provider job `{provider}`"
            ),
        }
    }
}

impl Error for JobSelectionError {}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionPlan, JobSelectionError, PlannedJob, PlannedPackage, PlannedProviderInstall,
        PlannedSingleProviderPackage,
    };
    use crate::job::{JobSelection, JobSelector};
    use crate::platform::PlatformInfo;
    use crate::schema::{Identifier, ResolvedExecAction, SelectorIdentifier};

    fn provider_id(value: &str) -> Identifier {
        Identifier::new(value).expect("test identifier should be valid")
    }

    fn selector_id(value: &str) -> SelectorIdentifier {
        SelectorIdentifier::new(value).expect("test selector identifier should be valid")
    }

    #[test]
    fn exact_provider_package_reports_its_missing_provider_job() {
        let package = selector_id("orphan");
        let provider = provider_id("missing");
        let plan = ExecutionPlan {
            target: String::from("test"),
            profile: None,
            platform: PlatformInfo::detect(),
            jobs: vec![PlannedJob::Package(PlannedPackage::Provider(
                PlannedProviderInstall::Single(PlannedSingleProviderPackage {
                    id: package.clone(),
                    provider: provider.clone(),
                    provider_args: Vec::new(),
                    install: ResolvedExecAction {
                        kind: None,
                        program: "unused".into(),
                        args: Vec::new(),
                        cwd: None,
                        env: None,
                    },
                }),
            ))],
        };

        let error = plan
            .select(&JobSelection::only(JobSelector::Package(package.clone())))
            .expect_err("selection should reject the missing provider job");

        assert!(matches!(
            error,
            JobSelectionError::MissingProvider {
                package: actual_package,
                provider: actual_provider,
            } if actual_package == package && actual_provider == provider
        ));
    }
}
