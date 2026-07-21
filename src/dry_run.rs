use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fmt::Write as _;
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
pub struct DryRunPlan {
    target: String,
    profile: Option<String>,
    platform: PlatformInfo,
    providers: Vec<PlannedProvider>,
    provider_batches: Vec<PlannedProviderBatch>,
    manual_packages: Vec<PlannedManualPackage>,
    actions: Vec<PlannedAction>,
    links: Vec<PlannedLink>,
}

impl DryRunPlan {
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

impl fmt::Display for DryRunPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut output = String::new();
        writeln!(output, "target: {}", self.target)?;
        writeln!(
            output,
            "profile: {}",
            self.profile.as_deref().unwrap_or("<root>")
        )?;
        writeln!(
            output,
            "platform: {}/{}",
            self.platform.os, self.platform.arch
        )?;
        if let Some(distro) = &self.platform.distro {
            writeln!(output, "distro: {distro}")?;
        }
        if !self.platform.distro_families.is_empty() {
            writeln!(
                output,
                "distro families: {:?}",
                self.platform.distro_families
            )?;
        }
        writeln!(output, "environments: {:?}", self.platform.environments)?;

        writeln!(output, "\nproviders:")?;
        if self.providers.is_empty() {
            writeln!(output, "  <none>")?;
        }
        for provider in &self.providers {
            writeln!(output, "  {}", provider.id)?;
            match &provider.activate {
                Some(activate) => {
                    writeln!(output, "    activate:")?;
                    write_environment_patch(&mut output, activate, "      ")?;
                }
                None => writeln!(output, "    activate: <none>")?,
            }
            write_exec_action(&mut output, "probe", &provider.probe, "    ")?;
            if provider.ensure.is_empty() {
                writeln!(output, "    ensure: <none>")?;
            } else {
                writeln!(output, "    ensure:")?;
                for (index, action) in provider.ensure.iter().enumerate() {
                    writeln!(output, "      [{index}]")?;
                    write_exec_fields(&mut output, action, "        ")?;
                }
            }
        }

        writeln!(output, "\nprovider packages:")?;
        if self.provider_batches.is_empty() {
            writeln!(output, "  <none>")?;
        }
        for batch in &self.provider_batches {
            writeln!(output, "  {}", batch.provider)?;
            writeln!(output, "    provider_args: {:?}", batch.provider_args)?;
            writeln!(output, "    packages: {:?}", batch.packages)?;
            write_exec_action(&mut output, "install", &batch.install, "    ")?;
        }

        writeln!(output, "\nmanual packages:")?;
        if self.manual_packages.is_empty() {
            writeln!(output, "  <none>")?;
        }
        for package in &self.manual_packages {
            writeln!(output, "  {}", package.id)?;
            write_action(&mut output, &package.install, "    ")?;
        }

        writeln!(output, "\nactions:")?;
        if self.actions.is_empty() {
            writeln!(output, "  <none>")?;
        }
        for action in &self.actions {
            writeln!(output, "  {}", action.id)?;
            write_action(&mut output, &action.action, "    ")?;
        }

        writeln!(output, "\nlinks:")?;
        if self.links.is_empty() {
            writeln!(output, "  <none>")?;
        }
        for link in &self.links {
            writeln!(
                output,
                "  {}: {:?} -> {:?}",
                link.id,
                link.source.display().to_string(),
                link.target.display().to_string()
            )?;
            writeln!(
                output,
                "    on_conflict: {}",
                link_conflict_name(link.on_conflict)
            )?;
            writeln!(
                output,
                "    on_missing_parent: {}",
                link_missing_parent_name(link.on_missing_parent)
            )?;
        }

        formatter.write_str(output.trim_end())
    }
}

fn write_action(output: &mut String, action: &Action, indent: &str) -> fmt::Result {
    match &action.check {
        Some(check) => write_exec_action(output, "check", check, indent)?,
        None => writeln!(output, "{indent}check: <none>")?,
    }
    write_exec_action(output, "exec", &action.exec, indent)
}

fn write_exec_action(
    output: &mut String,
    label: &str,
    action: &ExecAction,
    indent: &str,
) -> fmt::Result {
    writeln!(output, "{indent}{label}:")?;
    let field_indent = format!("{indent}  ");
    write_exec_fields(output, action, &field_indent)
}

fn write_exec_fields(output: &mut String, action: &ExecAction, indent: &str) -> fmt::Result {
    writeln!(output, "{indent}program: {:?}", action.program.as_str())?;
    let args = action
        .args
        .iter()
        .map(|argument| argument.as_str())
        .collect::<Vec<_>>();
    writeln!(output, "{indent}args: {args:?}")?;
    if let Some(cwd) = &action.cwd {
        writeln!(output, "{indent}cwd: {:?}", cwd.as_str())?;
    }
    if let Some(environment) = &action.env {
        writeln!(output, "{indent}env:")?;
        let environment_indent = format!("{indent}  ");
        write_environment_patch(output, environment, &environment_indent)?;
    }
    Ok(())
}

fn write_environment_patch(
    output: &mut String,
    patch: &EnvironmentPatch,
    indent: &str,
) -> fmt::Result {
    if let Some(values) = &patch.path_prepend {
        writeln!(output, "{indent}path_prepend: {:?}", scalar_values(values))?;
    }
    if let Some(values) = &patch.path_append {
        writeln!(output, "{indent}path_append: {:?}", scalar_values(values))?;
    }
    if !patch.variables.is_empty() {
        writeln!(output, "{indent}variables:")?;
        for (name, value) in &patch.variables {
            writeln!(output, "{indent}  {name}: {:?}", value.as_str())?;
        }
    }
    Ok(())
}

fn scalar_values(values: &OneOrMany<crate::schema::ScalarTemplate>) -> Vec<&str> {
    match values {
        OneOrMany::One(value) => vec![value.as_str()],
        OneOrMany::Many(values) => values.iter().map(|value| value.as_str()).collect(),
    }
}

fn link_conflict_name(value: LinkConflict) -> &'static str {
    match value {
        LinkConflict::Error => "error",
        LinkConflict::ReplaceLink => "replace-link",
    }
}

fn link_missing_parent_name(value: LinkMissingParent) -> &'static str {
    match value {
        LinkMissingParent::Create => "create",
        LinkMissingParent::Skip => "skip",
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
pub struct DryRunPlanner<'a> {
    base_environment: &'a ExecutionEnvironment,
    dot_paths: DotPaths<'a>,
    xdg_paths: &'a XdgPaths,
    platform: &'a PlatformInfo,
}

impl<'a> DryRunPlanner<'a> {
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

    pub fn plan(&self, manifest: &EffectiveManifest) -> Result<DryRunPlan, DryRunError> {
        let groups = self.group_provider_packages(manifest)?;
        let (providers, provider_environments) = self.plan_providers(manifest)?;
        let provider_batches =
            self.plan_provider_batches(manifest, groups, &provider_environments)?;
        let manual_packages = self.plan_manual_packages(manifest)?;
        let actions = self.plan_actions(manifest)?;
        let links = self.plan_links(manifest)?;

        Ok(DryRunPlan {
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
    ) -> Result<ProviderGroups, DryRunError> {
        let mut groups = BTreeMap::new();

        for (package_id, package) in manifest.packages() {
            let Package::Provider(package) = package else {
                continue;
            };
            let provider = package.provider.as_str();
            if !manifest.providers().contains_key(provider) {
                return Err(DryRunError::UnknownProvider {
                    package: package_id.to_string(),
                    provider: provider.to_owned(),
                });
            }

            let provider_args = package
                .provider_args
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(resolve_literal_string)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|source| DryRunError::Interpolation {
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
    ) -> Result<(Vec<PlannedProvider>, BTreeMap<String, ExecutionEnvironment>), DryRunError> {
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
                .map_err(|source| DryRunError::Interpolation {
                    context: format!("provider `{provider_id}` activate"),
                    source,
                })?;
            if let Some(activate) = &activate {
                environment.apply_patch(activate).map_err(|source| {
                    DryRunError::EnvironmentPatch {
                        provider: provider_id.to_string(),
                        source,
                    }
                })?;
            }

            let context = ResolveContext::new(&environment, self.dot_paths, self.xdg_paths);
            let probe = resolve_exec_action(&provider.probe, &context).map_err(|source| {
                DryRunError::Interpolation {
                    context: format!("provider `{provider_id}` probe"),
                    source,
                }
            })?;
            let ensure = resolve_ensure(provider, &context).map_err(|source| {
                DryRunError::Interpolation {
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
    ) -> Result<Vec<PlannedProviderBatch>, DryRunError> {
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
                        return Err(DryRunError::ProviderArgsResolverCount {
                            provider: provider_id,
                            actual: resolver_count,
                        });
                    }
                }
                let package_context = PackageContext::new(&packages, &provider_args);
                let context = ResolveContext::new(environment, self.dot_paths, self.xdg_paths)
                    .with_package(package_context);
                let install = resolve_provider_install_action(&provider.install, &context)
                    .map_err(|source| DryRunError::Interpolation {
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
    ) -> Result<Vec<PlannedManualPackage>, DryRunError> {
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
                        .map_err(|source| DryRunError::Interpolation {
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
    ) -> Result<Vec<PlannedAction>, DryRunError> {
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
                    .map_err(|source| DryRunError::Interpolation {
                        context: format!("action `{action_id}`"),
                        source,
                    })
            })
            .collect()
    }

    fn plan_links(&self, manifest: &EffectiveManifest) -> Result<Vec<PlannedLink>, DryRunError> {
        let context = ResolveContext::new(self.base_environment, self.dot_paths, self.xdg_paths);
        manifest
            .links()
            .iter()
            .map(|(link_id, link)| {
                let source = crate::interpolation::resolve_scalar_template(&link.source, &context)
                    .map_err(|source| DryRunError::Interpolation {
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
                    .map_err(|source| DryRunError::Interpolation {
                        context: format!("link `{link_id}` target"),
                        source,
                    })?;
                if !target.is_absolute() {
                    return Err(DryRunError::RelativeLinkTarget {
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
pub enum DryRunError {
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

impl fmt::Display for DryRunError {
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

impl Error for DryRunError {
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
