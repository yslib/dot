use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use crate::interpolation::{
    InterpolationError, promote_literal_string, promote_provider_install_args,
    promote_string_expression, provider_args_resolver_count,
};
use crate::manifest::{EffectiveManifest, ManifestError, profile_entries};
use crate::schema::{
    Action, EnvironmentPatch, ExecAction, Identifier, Link, OneOrMany, Package, Profile, Provider,
    ProviderInstallArgSource, ProviderPackage, SelectableEntries, SelectorIdentifier, SourceAction,
    SourceExecAction, StringExpressionSource, Target,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigValidationJob {
    Provider(Identifier),
    Package(SelectorIdentifier),
    Action(SelectorIdentifier),
    Link(SelectorIdentifier),
}

impl fmt::Display for ConfigValidationJob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Provider(id) => write!(formatter, "provider `{id}`"),
            Self::Package(id) => write!(formatter, "package `{id}`"),
            Self::Action(id) => write!(formatter, "action `{id}`"),
            Self::Link(id) => write!(formatter, "link `{id}`"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigValidationErrorKind {
    Expression(InterpolationError),
    UnknownProvider {
        package: SelectorIdentifier,
        provider: Identifier,
    },
    EmptyPackageBatch {
        package: SelectorIdentifier,
    },
    DuplicatePackageBatchName {
        package: SelectorIdentifier,
        name: Identifier,
    },
    ProviderArgsResolverCount {
        provider: Identifier,
        actual: usize,
    },
    Manifest(ManifestError),
}

impl fmt::Display for ConfigValidationErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expression(source) => source.fmt(formatter),
            Self::UnknownProvider { package, provider } => write!(
                formatter,
                "package `{package}` references unknown provider `{provider}`"
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
            Self::ProviderArgsResolverCount { provider, actual } => write!(
                formatter,
                "provider `{provider}` install must contain exactly one `${{package:provider_args}}` argument for an install unit with nonempty provider_args; found {actual}"
            ),
            Self::Manifest(source) => source.fmt(formatter),
        }
    }
}

impl Error for ConfigValidationErrorKind {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Expression(source) => Some(source),
            Self::Manifest(source) => Some(source),
            Self::UnknownProvider { .. }
            | Self::EmptyPackageBatch { .. }
            | Self::DuplicatePackageBatchName { .. }
            | Self::ProviderArgsResolverCount { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigValidationError {
    pub target: SelectorIdentifier,
    pub profile: Option<SelectorIdentifier>,
    pub job: Option<ConfigValidationJob>,
    pub field: Option<String>,
    pub kind: Box<ConfigValidationErrorKind>,
}

impl fmt::Display for ConfigValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "target `{}`", self.target)?;
        if let Some(profile) = &self.profile {
            write!(formatter, " profile `{profile}`")?;
        }
        if let Some(job) = &self.job {
            write!(formatter, " {job}")?;
        }
        if let Some(field) = &self.field {
            write!(formatter, " field `{field}`")?;
        }
        write!(formatter, ": {}", self.kind)
    }
}

impl Error for ConfigValidationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.kind.as_ref())
    }
}

#[derive(Clone)]
struct ValidationContext {
    target: SelectorIdentifier,
    profile: Option<SelectorIdentifier>,
    job: Option<ConfigValidationJob>,
}

impl ValidationContext {
    fn target(target: &SelectorIdentifier) -> Self {
        Self {
            target: target.clone(),
            profile: None,
            job: None,
        }
    }

    fn profile(&self, profile: Option<&SelectorIdentifier>) -> Self {
        Self {
            target: self.target.clone(),
            profile: profile.cloned(),
            job: None,
        }
    }

    fn job(&self, job: ConfigValidationJob) -> Self {
        Self {
            target: self.target.clone(),
            profile: self.profile.clone(),
            job: Some(job),
        }
    }

    fn error(
        &self,
        field: Option<impl Into<String>>,
        kind: ConfigValidationErrorKind,
    ) -> ConfigValidationError {
        ConfigValidationError {
            target: self.target.clone(),
            profile: self.profile.clone(),
            job: self.job.clone(),
            field: field.map(Into::into),
            kind: Box::new(kind),
        }
    }

    fn expression(
        &self,
        field: impl Into<String>,
        source: InterpolationError,
    ) -> ConfigValidationError {
        self.error(Some(field), ConfigValidationErrorKind::Expression(source))
    }
}

pub fn validate_config(config: &crate::schema::Config) -> Result<(), ConfigValidationError> {
    for (target_id, target) in &config.targets {
        let target_context = ValidationContext::target(target_id);
        let profiles = profile_entries(target_id, target)
            .map_err(|source| {
                target_context.error(None::<String>, ConfigValidationErrorKind::Manifest(source))
            })?
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();

        validate_scope(target, &target_context.profile(None))?;
        validate_profiles(&target.profiles, &target_context)?;

        validate_effective_scope(
            EffectiveManifest::from_declared_scope(target_id, target, None).map_err(|source| {
                target_context.error(None::<String>, ConfigValidationErrorKind::Manifest(source))
            })?,
            &target_context.profile(None),
        )?;

        for profile_id in profiles {
            let context = target_context.profile(Some(&profile_id));
            let manifest =
                EffectiveManifest::from_declared_scope(target_id, target, Some(&profile_id))
                    .map_err(|source| {
                        context.error(None::<String>, ConfigValidationErrorKind::Manifest(source))
                    })?;
            validate_effective_scope(manifest, &context)?;
        }
    }

    Ok(())
}

fn validate_profiles(
    profiles: &SelectableEntries<Profile>,
    target: &ValidationContext,
) -> Result<(), ConfigValidationError> {
    for (profile_id, profile) in profiles {
        validate_scope(profile, &target.profile(Some(profile_id)))?;
        validate_profiles(&profile.profiles, target)?;
    }
    Ok(())
}

trait ManifestScope {
    fn providers(&self) -> &crate::schema::Entries<Provider>;
    fn packages(&self) -> &SelectableEntries<Package>;
    fn links(&self) -> &SelectableEntries<Link>;
    fn actions(&self) -> &SelectableEntries<Action>;
}

impl ManifestScope for Target {
    fn providers(&self) -> &crate::schema::Entries<Provider> {
        &self.providers
    }

    fn packages(&self) -> &SelectableEntries<Package> {
        &self.packages
    }

    fn links(&self) -> &SelectableEntries<Link> {
        &self.links
    }

    fn actions(&self) -> &SelectableEntries<Action> {
        &self.actions
    }
}

impl ManifestScope for Profile {
    fn providers(&self) -> &crate::schema::Entries<Provider> {
        &self.providers
    }

    fn packages(&self) -> &SelectableEntries<Package> {
        &self.packages
    }

    fn links(&self) -> &SelectableEntries<Link> {
        &self.links
    }

    fn actions(&self) -> &SelectableEntries<Action> {
        &self.actions
    }
}

fn validate_scope(
    scope: &impl ManifestScope,
    context: &ValidationContext,
) -> Result<(), ConfigValidationError> {
    for (provider_id, provider) in scope.providers() {
        validate_provider(
            provider,
            &context.job(ConfigValidationJob::Provider(provider_id.clone())),
        )?;
    }
    for (package_id, package) in scope.packages() {
        validate_package(
            package,
            &context.job(ConfigValidationJob::Package(package_id.clone())),
        )?;
    }
    for (action_id, action) in scope.actions() {
        validate_action(
            action,
            &context.job(ConfigValidationJob::Action(action_id.clone())),
            "",
        )?;
    }
    for (link_id, link) in scope.links() {
        validate_link(
            link,
            &context.job(ConfigValidationJob::Link(link_id.clone())),
        )?;
    }
    Ok(())
}

fn validate_provider(
    provider: &Provider,
    context: &ValidationContext,
) -> Result<(), ConfigValidationError> {
    if let Some(activate) = &provider.activate {
        validate_environment_patch(activate, context, "activate")?;
    }
    validate_exec_action(&provider.probe, context, "probe")?;
    if let Some(ensure) = &provider.ensure {
        match ensure {
            OneOrMany::One(action) => validate_exec_action(action, context, "ensure")?,
            OneOrMany::Many(actions) => {
                for (index, action) in actions.iter().enumerate() {
                    validate_exec_action(action, context, &format!("ensure[{index}]"))?;
                }
            }
        }
    }
    validate_provider_install_action(&provider.install, context, "install")
}

fn validate_package(
    package: &Package,
    context: &ValidationContext,
) -> Result<(), ConfigValidationError> {
    match package {
        Package::Provider(package) => {
            if let Some(arguments) = package.provider_args() {
                for (index, argument) in arguments.iter().enumerate() {
                    promote_literal_string(argument).map_err(|source| {
                        context.expression(format!("provider_args[{index}]"), source)
                    })?;
                }
            }
            Ok(())
        }
        Package::Manual(package) => validate_action(&package.install, context, "install"),
    }
}

fn validate_link(link: &Link, context: &ValidationContext) -> Result<(), ConfigValidationError> {
    promote_string_expression(&link.source)
        .map_err(|source| context.expression("source", source))?;
    promote_string_expression(&link.target)
        .map_err(|source| context.expression("target", source))?;
    Ok(())
}

fn validate_action(
    action: &SourceAction,
    context: &ValidationContext,
    prefix: &str,
) -> Result<(), ConfigValidationError> {
    if let Some(check) = &action.check {
        validate_exec_action(check, context, &field(prefix, "check"))?;
    }
    validate_exec_action(&action.exec, context, &field(prefix, "exec"))
}

fn validate_exec_action(
    action: &SourceExecAction,
    context: &ValidationContext,
    prefix: &str,
) -> Result<(), ConfigValidationError> {
    promote_string_expression(&action.program)
        .map_err(|source| context.expression(field(prefix, "program"), source))?;
    for (index, argument) in action.args.iter().enumerate() {
        promote_string_expression(argument).map_err(|source| {
            context.expression(field(prefix, &format!("args[{index}]")), source)
        })?;
    }
    if let Some(cwd) = &action.cwd {
        promote_string_expression(cwd)
            .map_err(|source| context.expression(field(prefix, "cwd"), source))?;
    }
    if let Some(environment) = &action.env {
        validate_environment_patch(environment, context, &field(prefix, "env"))?;
    }
    Ok(())
}

fn validate_provider_install_action(
    action: &ExecAction<StringExpressionSource, ProviderInstallArgSource>,
    context: &ValidationContext,
    prefix: &str,
) -> Result<(), ConfigValidationError> {
    promote_string_expression(&action.program)
        .map_err(|source| context.expression(field(prefix, "program"), source))?;
    promote_provider_install_args(&action.args)
        .map_err(|source| context.expression(field(prefix, "args"), source))?;
    if let Some(cwd) = &action.cwd {
        promote_string_expression(cwd)
            .map_err(|source| context.expression(field(prefix, "cwd"), source))?;
    }
    if let Some(environment) = &action.env {
        validate_environment_patch(environment, context, &field(prefix, "env"))?;
    }
    Ok(())
}

fn validate_environment_patch(
    patch: &EnvironmentPatch,
    context: &ValidationContext,
    prefix: &str,
) -> Result<(), ConfigValidationError> {
    if let Some(values) = &patch.path_prepend {
        validate_expression_values(values, context, &field(prefix, "path_prepend"))?;
    }
    if let Some(values) = &patch.path_append {
        validate_expression_values(values, context, &field(prefix, "path_append"))?;
    }
    for (name, value) in &patch.variables {
        promote_string_expression(value).map_err(|source| {
            context.expression(field(prefix, &format!("variables.{name}")), source)
        })?;
    }
    Ok(())
}

fn validate_expression_values(
    values: &OneOrMany<StringExpressionSource>,
    context: &ValidationContext,
    field_name: &str,
) -> Result<(), ConfigValidationError> {
    match values {
        OneOrMany::One(value) => {
            promote_string_expression(value)
                .map_err(|source| context.expression(field_name, source))?;
        }
        OneOrMany::Many(values) => {
            for (index, value) in values.iter().enumerate() {
                promote_string_expression(value).map_err(|source| {
                    context.expression(format!("{field_name}[{index}]"), source)
                })?;
            }
        }
    }
    Ok(())
}

fn validate_effective_scope(
    manifest: EffectiveManifest,
    context: &ValidationContext,
) -> Result<(), ConfigValidationError> {
    for (package_id, package) in manifest.packages() {
        let Package::Provider(package) = package else {
            continue;
        };
        let package_context = context.job(ConfigValidationJob::Package(package_id.clone()));
        let provider_id = package.provider();
        let provider = manifest.providers().get(provider_id).ok_or_else(|| {
            package_context.error(
                Some("provider"),
                ConfigValidationErrorKind::UnknownProvider {
                    package: package_id.clone(),
                    provider: provider_id.clone(),
                },
            )
        })?;

        if let ProviderPackage::Batch(batch) = package {
            if batch.names.is_empty() {
                return Err(package_context.error(
                    Some("names"),
                    ConfigValidationErrorKind::EmptyPackageBatch {
                        package: package_id.clone(),
                    },
                ));
            }
            let mut seen = BTreeSet::new();
            for name in &batch.names {
                if !seen.insert(name) {
                    return Err(package_context.error(
                        Some("names"),
                        ConfigValidationErrorKind::DuplicatePackageBatchName {
                            package: package_id.clone(),
                            name: name.clone(),
                        },
                    ));
                }
            }
        }

        if package.provider_args().is_some_and(|args| !args.is_empty()) {
            let install_args = promote_provider_install_args(&provider.install.args)
                .map_err(|source| package_context.expression("provider.install.args", source))?;
            let actual = provider_args_resolver_count(&install_args);
            if actual != 1 {
                return Err(package_context.error(
                    Some("provider_args"),
                    ConfigValidationErrorKind::ProviderArgsResolverCount {
                        provider: provider_id.clone(),
                        actual,
                    },
                ));
            }
        }
    }
    Ok(())
}

fn field(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}.{name}")
    }
}
