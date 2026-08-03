use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use url::Url;

use crate::action::{CommandPreparationError, ExecutionEnvironment};
use crate::interpolation::{
    DotPaths, ExecActionResolutionError, InterpolationError, PackageContext, ResolveContext,
    XdgPaths, promote_provider_install_args, provider_args_resolver_count,
    resolve_environment_patch, resolve_exec_action_with_fields, resolve_literal_string,
    resolve_provider_install_action_with_args, resolve_string_expression,
};
use crate::job::{JobId, JobSelection, JobSelector};
use crate::manifest::EffectiveManifest;
use crate::platform::PlatformInfo;
use crate::schema::{
    Action, FetchContentAction, FetchContentConflict, Identifier, LinkConflict, LinkMissingParent,
    OneOrMany, Package, Provider, ProviderPackage, ResolvedCommandAction, ResolvedEnvironmentPatch,
    ResolvedExecAction, SelectorIdentifier, SourceCommandAction, SourceExecAction,
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
}

#[derive(Debug)]
pub enum PlannedPackage {
    Provider(PlannedProviderInstall),
    Manual(PlannedManualPackage),
}

impl PlannedPackage {
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
    install: ResolvedCommandAction,
}

impl PlannedManualPackage {
    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    pub(crate) fn job_id(&self) -> JobId {
        JobId::Package(self.id.clone())
    }

    pub fn install(&self) -> &ResolvedCommandAction {
        &self.install
    }
}

#[derive(Debug)]
pub struct PlannedAction {
    id: SelectorIdentifier,
    action: ResolvedCommandAction,
}

impl PlannedAction {
    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    pub(crate) fn job_id(&self) -> JobId {
        JobId::Action(self.id.clone())
    }

    pub fn action(&self) -> &ResolvedCommandAction {
        &self.action
    }
}

#[derive(Debug)]
pub struct PlannedFetchContentAction {
    source: Url,
    target: PathBuf,
    on_conflict: FetchContentConflict,
}

impl PlannedFetchContentAction {
    pub(crate) fn new(source: Url, target: PathBuf, on_conflict: FetchContentConflict) -> Self {
        Self {
            source,
            target,
            on_conflict,
        }
    }

    pub fn source(&self) -> &Url {
        &self.source
    }

    pub fn target(&self) -> &Path {
        &self.target
    }

    pub fn on_conflict(&self) -> FetchContentConflict {
        self.on_conflict
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionMode {
    All,
    Only,
}

#[derive(Debug)]
struct NormalizedJobSelection {
    mode: SelectionMode,
    packages: BTreeSet<SelectorIdentifier>,
    actions: BTreeSet<SelectorIdentifier>,
    links: BTreeSet<SelectorIdentifier>,
    providers: BTreeSet<Identifier>,
}

impl NormalizedJobSelection {
    fn new(
        manifest: &EffectiveManifest,
        requested: &JobSelection,
    ) -> Result<Self, JobSelectionError> {
        let JobSelection::Only(selectors) = requested else {
            return Ok(Self {
                mode: SelectionMode::All,
                packages: BTreeSet::new(),
                actions: BTreeSet::new(),
                links: BTreeSet::new(),
                providers: BTreeSet::new(),
            });
        };

        let mut normalized = Self {
            mode: SelectionMode::Only,
            packages: BTreeSet::new(),
            actions: BTreeSet::new(),
            links: BTreeSet::new(),
            providers: BTreeSet::new(),
        };
        let mut missing_provider = None;

        for selector in selectors {
            match selector {
                JobSelector::Package(package_id) => {
                    let package = manifest
                        .packages()
                        .get(package_id.as_str())
                        .ok_or_else(|| JobSelectionError::Unknown(selector.clone()))?;
                    normalized.packages.insert(package_id.clone());

                    if let Package::Provider(package) = package {
                        let provider = package.provider();
                        if manifest.providers().contains_key(provider.as_str()) {
                            normalized.providers.insert(provider.clone());
                        } else if missing_provider.is_none() {
                            missing_provider = Some(JobSelectionError::MissingProvider {
                                package: package_id.clone(),
                                provider: provider.clone(),
                            });
                        }
                    }
                }
                JobSelector::Action(action_id) => {
                    if !manifest.actions().contains_key(action_id.as_str()) {
                        return Err(JobSelectionError::Unknown(selector.clone()));
                    }
                    normalized.actions.insert(action_id.clone());
                }
                JobSelector::Link(link_id) => {
                    if !manifest.links().contains_key(link_id.as_str()) {
                        return Err(JobSelectionError::Unknown(selector.clone()));
                    }
                    normalized.links.insert(link_id.clone());
                }
            }
        }

        if let Some(error) = missing_provider {
            return Err(error);
        }
        Ok(normalized)
    }

    fn includes_package(&self, id: &SelectorIdentifier) -> bool {
        self.mode == SelectionMode::All || self.packages.contains(id)
    }

    fn includes_action(&self, id: &SelectorIdentifier) -> bool {
        self.mode == SelectionMode::All || self.actions.contains(id)
    }

    fn includes_link(&self, id: &SelectorIdentifier) -> bool {
        self.mode == SelectionMode::All || self.links.contains(id)
    }

    fn includes_provider(&self, id: &Identifier) -> bool {
        self.mode == SelectionMode::All || self.providers.contains(id)
    }
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

    pub fn plan(
        &self,
        manifest: &EffectiveManifest,
        selection: &JobSelection,
    ) -> Result<ExecutionPlan, ExecutionPlanError> {
        let selection = NormalizedJobSelection::new(manifest, selection)?;
        let (providers, provider_environments) = self.plan_providers(manifest, &selection)?;
        let provider_installs =
            self.plan_provider_installs(manifest, &provider_environments, &selection)?;
        let manual_packages = self.plan_manual_packages(manifest, &selection)?;
        let actions = self.plan_actions(manifest, &selection)?;
        let links = self.plan_links(manifest, &selection)?;

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
        selection: &NormalizedJobSelection,
    ) -> Result<(Vec<PlannedProvider>, BTreeMap<String, ExecutionEnvironment>), PlanningError> {
        let mut plans = Vec::new();
        let mut environments = BTreeMap::new();

        for (provider_id, provider) in manifest.providers() {
            if !selection.includes_provider(provider_id) {
                continue;
            }
            let job_id = JobId::Provider(provider_id.clone());
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
                    context: selected_field_context(&job_id, "activate"),
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
            let probe = resolve_exec_action_fields(&provider.probe, &context, "probe")
                .map_err(|error| selected_interpolation_error(&job_id, error))?;
            let ensure = resolve_ensure(provider, &context)
                .map_err(|error| selected_interpolation_error(&job_id, error))?;

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
        selection: &NormalizedJobSelection,
    ) -> Result<Vec<PlannedProviderInstall>, PlanningError> {
        manifest
            .packages()
            .iter()
            .filter_map(|(package_id, package)| {
                if !selection.includes_package(package_id) {
                    return None;
                }
                let Package::Provider(package) = package else {
                    return None;
                };

                Some((|| {
                    let job_id = JobId::Package(package_id.clone());
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
                            context: selected_field_context(&job_id, "provider_args"),
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
                            context: selected_field_context(&job_id, "provider.install.args"),
                            source,
                        })?;
                    if !provider_args.is_empty() {
                        let resolver_count = provider_args_resolver_count(&install_args);
                        if resolver_count != 1 {
                            return Err(PlanningError::ProviderArgsResolverCount {
                                package: package_id.to_string(),
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
                    .map_err(|error| exec_action_field_error(error, "provider.install"))
                    .map_err(|error| selected_interpolation_error(&job_id, error))?;

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
        selection: &NormalizedJobSelection,
    ) -> Result<Vec<PlannedManualPackage>, PlanningError> {
        let context = ResolveContext::new(self.base_environment, self.dot_paths, self.xdg_paths);
        manifest
            .packages()
            .iter()
            .filter_map(|(package_id, package)| {
                if !selection.includes_package(package_id) {
                    return None;
                }
                let Package::Manual(package) = package else {
                    return None;
                };
                Some(
                    resolve_command_action_fields(&package.install, &context, "install")
                        .map(|install| PlannedManualPackage {
                            id: package_id.clone(),
                            install,
                        })
                        .map_err(|error| {
                            selected_interpolation_error(&JobId::Package(package_id.clone()), error)
                        }),
                )
            })
            .collect()
    }

    fn plan_actions(
        &self,
        manifest: &EffectiveManifest,
        selection: &NormalizedJobSelection,
    ) -> Result<Vec<PlannedAction>, PlanningError> {
        let context = ResolveContext::new(self.base_environment, self.dot_paths, self.xdg_paths);
        manifest
            .actions()
            .iter()
            .filter(|(action_id, _)| selection.includes_action(action_id))
            .map(|(action_id, action)| {
                let Action::Command(action) = action else {
                    return Err(PlanningError::FetchContentNotYetWired {
                        action: action_id.to_string(),
                    });
                };
                resolve_command_action_fields(action, &context, "")
                    .map(|action| PlannedAction {
                        id: action_id.clone(),
                        action,
                    })
                    .map_err(|error| {
                        selected_interpolation_error(&JobId::Action(action_id.clone()), error)
                    })
            })
            .collect()
    }

    fn plan_links(
        &self,
        manifest: &EffectiveManifest,
        selection: &NormalizedJobSelection,
    ) -> Result<Vec<PlannedLink>, PlanningError> {
        let context = ResolveContext::new(self.base_environment, self.dot_paths, self.xdg_paths);
        manifest
            .links()
            .iter()
            .filter(|(link_id, _)| selection.includes_link(link_id))
            .map(|(link_id, link)| {
                let source =
                    resolve_string_expression(&link.source, &context).map_err(|source| {
                        PlanningError::Interpolation {
                            context: selected_field_context(
                                &JobId::Link(link_id.clone()),
                                "source",
                            ),
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
                        context: selected_field_context(&JobId::Link(link_id.clone()), "target"),
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

#[derive(Debug)]
struct FieldInterpolationError {
    field: String,
    source: InterpolationError,
}

fn selected_field_context(job: &JobId, field: &str) -> String {
    format!("selected job `{job}` field `{field}`")
}

fn selected_interpolation_error(job: &JobId, error: FieldInterpolationError) -> PlanningError {
    PlanningError::Interpolation {
        context: selected_field_context(job, &error.field),
        source: error.source,
    }
}

fn exec_action_field_error(
    error: ExecActionResolutionError,
    prefix: &str,
) -> FieldInterpolationError {
    let (field, source) = match error {
        ExecActionResolutionError::Program(source) => (nested_field(prefix, "program"), source),
        ExecActionResolutionError::Argument { index, source } => {
            (format!("{}[{index}]", nested_field(prefix, "args")), source)
        }
        ExecActionResolutionError::Cwd(source) => (nested_field(prefix, "cwd"), source),
        ExecActionResolutionError::Env(source) => (nested_field(prefix, "env"), source),
    };
    FieldInterpolationError { field, source }
}

fn resolve_command_action_fields(
    action: &SourceCommandAction,
    context: &ResolveContext<'_>,
    prefix: &str,
) -> Result<ResolvedCommandAction, FieldInterpolationError> {
    Ok(ResolvedCommandAction {
        check: action
            .check
            .as_ref()
            .map(|check| resolve_exec_action_fields(check, context, &nested_field(prefix, "check")))
            .transpose()?,
        exec: resolve_exec_action_fields(&action.exec, context, &nested_field(prefix, "exec"))?,
    })
}

fn resolve_exec_action_fields(
    action: &SourceExecAction,
    context: &ResolveContext<'_>,
    prefix: &str,
) -> Result<ResolvedExecAction, FieldInterpolationError> {
    resolve_exec_action_with_fields(action, context)
        .map_err(|error| exec_action_field_error(error, prefix))
}

pub(crate) fn resolve_fetch_content_fields(
    action_id: &SelectorIdentifier,
    action: &FetchContentAction,
    context: &ResolveContext<'_>,
    config_dir: &Path,
) -> Result<PlannedFetchContentAction, PlanningError> {
    let action_name = action_id.to_string();
    let job = JobId::Action(action_id.clone());
    let source = resolve_string_expression(&action.source, context).map_err(|source| {
        PlanningError::Interpolation {
            context: selected_field_context(&job, "source"),
            source,
        }
    })?;
    let target = resolve_string_expression(&action.target, context).map_err(|source| {
        PlanningError::Interpolation {
            context: selected_field_context(&job, "target"),
            source,
        }
    })?;

    Ok(PlannedFetchContentAction::new(
        resolve_fetch_source(&action_name, source.value())?,
        resolve_fetch_target(&action_name, target.value(), config_dir)?,
        action.on_conflict.unwrap_or_default(),
    ))
}

const _: () = {
    let _ = resolve_fetch_content_fields;
};

fn resolve_fetch_source(action: &str, value: &str) -> Result<Url, PlanningError> {
    let source =
        Url::parse(value).map_err(|source| PlanningError::InvalidFetchContentSourceUrl {
            action: action.to_owned(),
            value: value.to_owned(),
            source,
        })?;
    let missing_https_host = source.scheme() == "https"
        && value.split_once(':').is_some_and(|(scheme, remainder)| {
            scheme.eq_ignore_ascii_case("https")
                && (remainder.starts_with("///") || remainder == "//")
        });
    if source.scheme() != "https" || source.host_str().is_none() || missing_https_host {
        return Err(PlanningError::UnsupportedFetchContentSource {
            action: action.to_owned(),
            source_url: source,
        });
    }
    if !source.username().is_empty() || source.password().is_some() {
        return Err(PlanningError::AuthenticatedFetchContentSource {
            action: action.to_owned(),
            source_url: source,
        });
    }
    Ok(source)
}

fn resolve_fetch_target(
    action: &str,
    value: &str,
    config_dir: &Path,
) -> Result<PathBuf, PlanningError> {
    let target = PathBuf::from(value);
    if target.is_absolute() {
        return Ok(target);
    }
    if Url::parse(value).is_ok() {
        return Err(PlanningError::UnsupportedFetchContentTarget {
            action: action.to_owned(),
            target: value.to_owned(),
        });
    }
    Ok(config_dir.join(target))
}

fn nested_field(prefix: &str, field: &str) -> String {
    if prefix.is_empty() {
        field.to_owned()
    } else {
        format!("{prefix}.{field}")
    }
}

fn resolve_ensure(
    provider: &Provider,
    context: &ResolveContext<'_>,
) -> Result<Vec<ResolvedExecAction>, FieldInterpolationError> {
    match &provider.ensure {
        None => Ok(Vec::new()),
        Some(OneOrMany::One(action)) => {
            Ok(vec![resolve_exec_action_fields(action, context, "ensure")?])
        }
        Some(OneOrMany::Many(actions)) => actions
            .iter()
            .enumerate()
            .map(|(index, action)| {
                resolve_exec_action_fields(action, context, &format!("ensure[{index}]"))
            })
            .collect(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlanningError {
    #[error(
        "selected job `package:{package}` field `provider` references unknown provider `{provider}`"
    )]
    UnknownProvider { package: String, provider: String },
    #[error("failed to resolve {context}: {source}")]
    Interpolation {
        context: String,
        #[source]
        source: InterpolationError,
    },
    #[error("failed to apply selected job `provider:{provider}` field `activate`: {source}")]
    EnvironmentPatch {
        provider: String,
        #[source]
        source: CommandPreparationError,
    },
    #[error(
        "selected job `package:{package}` field `provider.install.args` from provider `{provider}` must contain exactly one `${{package:provider_args}}` argument for nonempty provider_args; found {actual}"
    )]
    ProviderArgsResolverCount {
        package: String,
        provider: String,
        actual: usize,
    },
    #[error("selected job `package:{package}` field `names` must contain at least one name")]
    EmptyPackageBatch { package: String },
    #[error("selected job `package:{package}` field `names` contains duplicate name `{name}`")]
    DuplicatePackageBatchName { package: String, name: String },
    #[error(
        "selected job `action:{action}` is a fetch content action that is not yet wired for planning"
    )]
    FetchContentNotYetWired { action: String },
    #[error(
        "selected job `action:{action}` field `source` contains an invalid URL `{value}`: {source}"
    )]
    InvalidFetchContentSourceUrl {
        action: String,
        value: String,
        #[source]
        source: url::ParseError,
    },
    #[error(
        "selected job `action:{action}` field `source` must be a public HTTPS URL with a host: `{source_url}`"
    )]
    UnsupportedFetchContentSource { action: String, source_url: Url },
    #[error(
        "selected job `action:{action}` field `source` must not include URL userinfo: `{source_url}`"
    )]
    AuthenticatedFetchContentSource { action: String, source_url: Url },
    #[error(
        "selected job `action:{action}` field `target` must be a native local path, not URL `{target}`"
    )]
    UnsupportedFetchContentTarget { action: String, target: String },
    #[error(
        "selected job `link:{link}` field `target` must be absolute after interpolation: `{}`",
        .target.display()
    )]
    RelativeLinkTarget { link: String, target: PathBuf },
}

#[derive(Debug, thiserror::Error)]
pub enum JobSelectionError {
    #[error("unknown job `{0}`")]
    Unknown(JobSelector),

    #[error("package job `{package}` references missing provider job `{provider}`")]
    MissingProvider {
        package: SelectorIdentifier,
        provider: Identifier,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionPlanError {
    #[error("{0}")]
    Selection(#[from] JobSelectionError),

    #[error("{0}")]
    Planning(#[from] PlanningError),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use crate::action::ExecutionEnvironment;
    use crate::interpolation::{DotPaths, ResolveContext, XdgPaths};
    use crate::schema::{
        EnvironmentName, FetchContentAction, FetchContentConflict, ResolvedEnvironmentPatch,
        ResolvedString, SelectorIdentifier, StringExpressionSource,
    };

    use super::{PlanningError, resolve_fetch_content_fields};

    fn environment(variables: &[(&str, &str)]) -> ExecutionEnvironment {
        let patch = ResolvedEnvironmentPatch {
            path_prepend: None,
            path_append: None,
            variables: variables
                .iter()
                .map(|(name, value)| {
                    (
                        EnvironmentName::new(*name).expect("test variable name should be valid"),
                        ResolvedString::from(*value),
                    )
                })
                .collect::<BTreeMap<_, _>>(),
        };
        let mut environment = ExecutionEnvironment::empty();
        environment
            .apply_patch(&patch)
            .expect("test environment patch should apply");
        environment
    }

    fn dot_paths<'a>(entry_dir: &'a Path, entity_dir: &'a Path) -> DotPaths<'a> {
        DotPaths::new(entry_dir, entry_dir, entity_dir, entity_dir, entry_dir)
    }

    fn fetch_action(
        source: &str,
        target: &str,
        on_conflict: Option<FetchContentConflict>,
    ) -> FetchContentAction {
        FetchContentAction {
            source: StringExpressionSource::from(source),
            target: StringExpressionSource::from(target),
            on_conflict,
        }
    }

    fn resolve_fetch(
        action: &FetchContentAction,
        environment: &ExecutionEnvironment,
        xdg: &XdgPaths,
        entry_dir: &Path,
        entity_dir: &Path,
    ) -> Result<super::PlannedFetchContentAction, PlanningError> {
        let context = ResolveContext::new(environment, dot_paths(entry_dir, entity_dir), xdg);
        resolve_fetch_content_fields(
            &SelectorIdentifier::new("download").expect("test action id should be valid"),
            action,
            &context,
            entry_dir,
        )
    }

    mod fetch_content {
        use super::*;

        #[test]
        fn accepts_a_public_absolute_https_source() {
            let entry_dir = std::env::temp_dir().join("dot-fetch-entry");
            let entity_dir = std::env::temp_dir().join("dot-fetch-entity");
            let action = fetch_action("https://example.test/config.toml", "configs/app.toml", None);

            let planned = resolve_fetch(
                &action,
                &environment(&[]),
                &XdgPaths::default(),
                &entry_dir,
                &entity_dir,
            )
            .expect("public HTTPS source should resolve");

            assert_eq!(
                planned.source().as_str(),
                "https://example.test/config.toml"
            );
        }

        #[test]
        fn defaults_conflict_policy_to_error() {
            let entry_dir = std::env::temp_dir().join("dot-fetch-entry");
            let entity_dir = std::env::temp_dir().join("dot-fetch-entity");
            let action = fetch_action("https://example.test/config.toml", "config.toml", None);

            let planned = resolve_fetch(
                &action,
                &environment(&[]),
                &XdgPaths::default(),
                &entry_dir,
                &entity_dir,
            )
            .expect("fetch action should resolve");

            assert_eq!(planned.on_conflict(), FetchContentConflict::Error);
        }

        #[test]
        fn preserves_explicit_conflict_policy() {
            let entry_dir = std::env::temp_dir().join("dot-fetch-entry");
            let entity_dir = std::env::temp_dir().join("dot-fetch-entity");

            for conflict in [FetchContentConflict::Error, FetchContentConflict::Replace] {
                let action = fetch_action(
                    "https://example.test/config.toml",
                    "config.toml",
                    Some(conflict),
                );
                let planned = resolve_fetch(
                    &action,
                    &environment(&[]),
                    &XdgPaths::default(),
                    &entry_dir,
                    &entity_dir,
                )
                .expect("fetch action should resolve");

                assert_eq!(planned.on_conflict(), conflict);
            }
        }

        #[test]
        fn retains_an_absolute_native_target() {
            let entry_dir = std::env::temp_dir().join("dot-fetch-entry");
            let entity_dir = std::env::temp_dir().join("dot-fetch-entity");
            let target = std::env::temp_dir().join("dot-fetch-absolute-target.toml");
            let action = fetch_action(
                "https://example.test/config.toml",
                &target.to_string_lossy(),
                None,
            );

            let planned = resolve_fetch(
                &action,
                &environment(&[]),
                &XdgPaths::default(),
                &entry_dir,
                &entity_dir,
            )
            .expect("absolute native target should resolve");

            assert_eq!(planned.target(), target);
        }

        #[test]
        fn joins_a_relative_target_to_the_config_entry_directory() {
            let entry_dir = std::env::temp_dir().join("dot-fetch-entry");
            let entity_dir = std::env::temp_dir().join("dot-fetch-entity");
            let action = fetch_action("https://example.test/config.toml", "configs/app.toml", None);

            let planned = resolve_fetch(
                &action,
                &environment(&[]),
                &XdgPaths::default(),
                &entry_dir,
                &entity_dir,
            )
            .expect("relative target should resolve");

            assert_eq!(planned.target(), entry_dir.join("configs/app.toml"));
        }

        #[test]
        fn resolves_real_config_directory_targets_from_the_entity_directory() {
            let entry_dir = std::env::temp_dir().join("dot-fetch-entry");
            let entity_dir = std::env::temp_dir().join("dot-fetch-entity");
            let action = fetch_action(
                "https://example.test/config.toml",
                "${dot:real_config_dir}/configs/app.toml",
                None,
            );

            let planned = resolve_fetch(
                &action,
                &environment(&[]),
                &XdgPaths::default(),
                &entry_dir,
                &entity_dir,
            )
            .expect("dot real config directory target should resolve");

            assert_eq!(planned.target(), entity_dir.join("configs/app.toml"));
            assert_ne!(planned.target(), entry_dir.join("configs/app.toml"));
        }

        #[test]
        fn resolves_environment_and_xdg_expressions() {
            let entry_dir = std::env::temp_dir().join("dot-fetch-entry");
            let entity_dir = std::env::temp_dir().join("dot-fetch-entity");
            let action = fetch_action("${env:FETCH_SOURCE}", "${xdg:home}/dot-fetch.toml", None);

            let planned = resolve_fetch(
                &action,
                &environment(&[("FETCH_SOURCE", "https://example.test/config.toml")]),
                &XdgPaths::detect(),
                &entry_dir,
                &entity_dir,
            )
            .expect("environment and XDG expressions should resolve");

            assert_eq!(
                planned.source().as_str(),
                "https://example.test/config.toml"
            );
            assert!(planned.target().is_absolute());
            assert!(planned.target().ends_with("dot-fetch.toml"));
        }

        #[test]
        fn reports_source_environment_interpolation_with_canonical_action_context() {
            let entry_dir = std::env::temp_dir().join("dot-fetch-entry");
            let entity_dir = std::env::temp_dir().join("dot-fetch-entity");
            let error = resolve_fetch(
                &fetch_action("${env:MISSING_SOURCE}", "config.toml", None),
                &environment(&[]),
                &XdgPaths::default(),
                &entry_dir,
                &entity_dir,
            )
            .expect_err("missing source environment variable should fail");

            assert_eq!(
                error.to_string(),
                "failed to resolve selected job `action:download` field `source`: environment variable `MISSING_SOURCE` is not defined"
            );
        }

        #[test]
        fn reports_target_dot_interpolation_with_canonical_action_context() {
            let entry_dir = std::env::temp_dir().join("dot-fetch-entry");
            let entity_dir = std::env::temp_dir().join("dot-fetch-entity");
            let error = resolve_fetch(
                &fetch_action(
                    "https://example.test/config.toml",
                    "${dot:not_a_path}",
                    None,
                ),
                &environment(&[]),
                &XdgPaths::default(),
                &entry_dir,
                &entity_dir,
            )
            .expect_err("invalid dot resolver payload should fail");

            assert_eq!(
                error.to_string(),
                "failed to resolve selected job `action:download` field `target`: invalid payload `not_a_path` for resolver `dot`"
            );
        }

        #[test]
        fn reports_target_xdg_interpolation_with_canonical_action_context() {
            let entry_dir = std::env::temp_dir().join("dot-fetch-entry");
            let entity_dir = std::env::temp_dir().join("dot-fetch-entity");
            let error = resolve_fetch(
                &fetch_action(
                    "https://example.test/config.toml",
                    "${xdg:home}/dot-fetch.toml",
                    None,
                ),
                &environment(&[]),
                &XdgPaths::default(),
                &entry_dir,
                &entity_dir,
            )
            .expect_err("unavailable XDG path should fail");

            assert_eq!(
                error.to_string(),
                "failed to resolve selected job `action:download` field `target`: path value `home` is unavailable"
            );
        }

        #[test]
        fn rejects_disallowed_source_locators() {
            let entry_dir = std::env::temp_dir().join("dot-fetch-entry");
            let entity_dir = std::env::temp_dir().join("dot-fetch-entity");

            for (source, expected) in [
                (
                    "http://example.test/config.toml",
                    "unsupported non-HTTPS source",
                ),
                ("configs/app.toml", "invalid relative source"),
                ("https://[::1", "malformed source"),
                ("https:///config.toml", "missing-host source"),
                (
                    "https://user:password@example.test/config.toml",
                    "authenticated source",
                ),
            ] {
                let error = resolve_fetch(
                    &fetch_action(source, "config.toml", None),
                    &environment(&[]),
                    &XdgPaths::default(),
                    &entry_dir,
                    &entity_dir,
                )
                .expect_err("disallowed source locator should fail");

                assert!(
                    error
                        .to_string()
                        .contains("selected job `action:download` field `source`"),
                    "source `{source}` should identify the canonical source field: {error}"
                );
                match expected {
                    "unsupported non-HTTPS source" | "missing-host source" => {
                        let PlanningError::UnsupportedFetchContentSource { action, .. } = error
                        else {
                            panic!("source `{source}` should reject as unsupported")
                        };
                        assert_eq!(action, "download");
                    }
                    "invalid relative source" | "malformed source" => {
                        let PlanningError::InvalidFetchContentSourceUrl { action, value, .. } =
                            error
                        else {
                            panic!("source `{source}` should reject as malformed")
                        };
                        assert_eq!(action, "download");
                        assert_eq!(value, source);
                    }
                    "authenticated source" => {
                        let PlanningError::AuthenticatedFetchContentSource { action, .. } = error
                        else {
                            panic!("source `{source}` should reject as authenticated")
                        };
                        assert_eq!(action, "download");
                    }
                    _ => unreachable!("test source has an expected error variant"),
                }
            }
        }

        #[test]
        fn rejects_url_targets() {
            let entry_dir = std::env::temp_dir().join("dot-fetch-entry");
            let entity_dir = std::env::temp_dir().join("dot-fetch-entity");
            let error = resolve_fetch(
                &fetch_action(
                    "https://example.test/config.toml",
                    "https://example.test/config.toml",
                    None,
                ),
                &environment(&[]),
                &XdgPaths::default(),
                &entry_dir,
                &entity_dir,
            )
            .expect_err("URL target should fail");

            assert_eq!(
                error.to_string(),
                "selected job `action:download` field `target` must be a native local path, not URL `https://example.test/config.toml`"
            );
            let PlanningError::UnsupportedFetchContentTarget { action, target } = error else {
                panic!("URL target should reject as unsupported")
            };
            assert_eq!(action, "download");
            assert_eq!(target, "https://example.test/config.toml");
        }
    }
}
