//! Static validation and pure evaluation of configuration expressions.

mod environment;
mod resolver;

pub use environment::ExecutionEnvironment;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::ConfigFile;
use crate::schema::{
    EnvironmentPatch, ExecAction, ExpressionParseError, FlatListPart, ListType, LiteralString,
    LiteralStringSource, OneOrMany, ParsedStringForm, ParsedTemplate as ParsedStringTemplate,
    ParsedTemplatePart, ProviderInstallArgSource, ProviderInstallArgs, ResolvedEnvironmentPatch,
    ResolvedExecAction, ResolvedString, SchemaType, SchemaTypeMarker, SourceExecAction,
    StringExpression, StringExpressionSource, StringTemplate, StringTemplatePart, StringType,
    TypedVariable, UntypedVariableReference,
};

use resolver::{ResolverEntry, ResolverKind, lookup_resolver};

#[derive(Clone, Copy, Debug)]
pub struct DotPaths<'a> {
    config_dir: &'a Path,
    real_config_dir: &'a Path,
    cwd: &'a Path,
}

impl<'a> DotPaths<'a> {
    pub const fn new(config_dir: &'a Path, real_config_dir: &'a Path, cwd: &'a Path) -> Self {
        Self {
            config_dir,
            real_config_dir,
            cwd,
        }
    }

    pub const fn config_dir(&self) -> &'a Path {
        self.config_dir
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DotPath {
    ConfigDir,
    RealConfigDir,
    Cwd,
}

impl DotPath {
    fn from_payload(payload: &str) -> Option<Self> {
        match payload {
            "config_dir" => Some(Self::ConfigDir),
            "real_config_dir" => Some(Self::RealConfigDir),
            "cwd" => Some(Self::Cwd),
            _ => None,
        }
    }
}

impl DotPaths<'_> {
    fn get(&self, path: DotPath) -> &Path {
        match path {
            DotPath::ConfigDir => self.config_dir,
            DotPath::RealConfigDir => self.real_config_dir,
            DotPath::Cwd => self.cwd,
        }
    }
}

impl<'a> From<&'a ConfigFile> for DotPaths<'a> {
    fn from(config: &'a ConfigFile) -> Self {
        Self::new(config.config_dir(), config.real_config_dir(), config.cwd())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum XdgPath {
    Home,
    Config,
    ConfigLocal,
    Data,
    DataLocal,
    Cache,
    State,
    Runtime,
    Executable,
    Documents,
}

impl XdgPath {
    fn from_payload(payload: &str) -> Option<Self> {
        match payload {
            "home" => Some(Self::Home),
            "config" => Some(Self::Config),
            "config_local" => Some(Self::ConfigLocal),
            "data" => Some(Self::Data),
            "data_local" => Some(Self::DataLocal),
            "cache" => Some(Self::Cache),
            "state" => Some(Self::State),
            "runtime" => Some(Self::Runtime),
            "executable" => Some(Self::Executable),
            "documents" => Some(Self::Documents),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct XdgPaths {
    pub(crate) values: BTreeMap<XdgPath, PathBuf>,
}

impl XdgPaths {
    fn get(&self, path: XdgPath) -> Option<&Path> {
        self.values.get(&path).map(PathBuf::as_path)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PackageContext<'a> {
    names: &'a [String],
    provider_args: &'a [String],
}

impl<'a> PackageContext<'a> {
    pub const fn new(names: &'a [String], provider_args: &'a [String]) -> Self {
        Self {
            names,
            provider_args,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ResolveContext<'a> {
    environment: &'a ExecutionEnvironment,
    dot: DotPaths<'a>,
    xdg: &'a XdgPaths,
    package: Option<PackageContext<'a>>,
}

impl<'a> ResolveContext<'a> {
    pub const fn new(
        environment: &'a ExecutionEnvironment,
        dot: DotPaths<'a>,
        xdg: &'a XdgPaths,
    ) -> Self {
        Self {
            environment,
            dot,
            xdg,
            package: None,
        }
    }

    pub const fn with_package(mut self, package: PackageContext<'a>) -> Self {
        self.package = Some(package);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ResolvedValue {
    String(ResolvedString),
    StringList(Vec<ResolvedString>),
}

impl ResolvedValue {
    fn schema_type(&self) -> SchemaType {
        match self {
            Self::String(_) => StringType::schema_type(),
            Self::StringList(_) => ListType::<StringType>::schema_type(),
        }
    }

    fn into_string(self, resolver: &str) -> Result<ResolvedString, InterpolationError> {
        match self {
            Self::String(value) => Ok(value),
            other => Err(InterpolationError::ResolverContractViolation {
                resolver: resolver.to_owned(),
                expected: StringType::schema_type(),
                actual: other.schema_type(),
            }),
        }
    }

    fn into_string_list(self, resolver: &str) -> Result<Vec<ResolvedString>, InterpolationError> {
        match self {
            Self::StringList(values) => Ok(values),
            other => Err(InterpolationError::ResolverContractViolation {
                resolver: resolver.to_owned(),
                expected: ListType::<StringType>::schema_type(),
                actual: other.schema_type(),
            }),
        }
    }
}

fn resolve_variable(
    namespace: &str,
    payload: &str,
    context: &ResolveContext<'_>,
) -> Result<ResolvedValue, InterpolationError> {
    let resolver = lookup_resolver(namespace).expect("typed resolver exists");
    let value = match resolver.kind() {
        ResolverKind::Environment => {
            let value = context.environment.get(payload).ok_or_else(|| {
                InterpolationError::MissingEnvironmentVariable {
                    name: payload.to_owned(),
                }
            })?;
            let value = value.to_str().ok_or_else(|| {
                InterpolationError::NonUnicodeEnvironmentVariable {
                    name: payload.to_owned(),
                }
            })?;
            ResolvedValue::String(ResolvedString::from(value))
        }
        ResolverKind::DotPath => {
            let path = context.dot.get(
                DotPath::from_payload(payload)
                    .expect("payload was validated by the resolver definition"),
            );
            let value = path
                .to_str()
                .ok_or_else(|| InterpolationError::NonUnicodePath {
                    name: payload.to_owned(),
                })?;
            ResolvedValue::String(ResolvedString::from(value))
        }
        ResolverKind::XdgPath => {
            let path = XdgPath::from_payload(payload)
                .and_then(|path| context.xdg.get(path))
                .ok_or_else(|| InterpolationError::UnavailablePath {
                    name: payload.to_owned(),
                })?;
            let value = path
                .to_str()
                .ok_or_else(|| InterpolationError::NonUnicodePath {
                    name: payload.to_owned(),
                })?;
            ResolvedValue::String(ResolvedString::from(value))
        }
        ResolverKind::Package => {
            let package = context
                .package
                .ok_or(InterpolationError::MissingPackageContext)?;
            let values = match payload {
                "names" => package.names,
                "provider_args" => package.provider_args,
                _ => unreachable!("payload was validated by the resolver definition"),
            };
            ResolvedValue::StringList(values.iter().cloned().map(ResolvedString::from).collect())
        }
    };

    let actual = value.schema_type();
    if &actual != resolver.output_type() {
        return Err(InterpolationError::ResolverContractViolation {
            resolver: namespace.to_owned(),
            expected: resolver.output_type().clone(),
            actual,
        });
    }
    Ok(value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TemplateRole {
    Scalar,
    ProviderInstallArg,
}

fn validate_resolver_reference(
    reference: &UntypedVariableReference,
    role: TemplateRole,
) -> Result<&'static ResolverEntry, InterpolationError> {
    let resolver = lookup_resolver(reference.resolver()).ok_or_else(|| {
        InterpolationError::UnknownResolver {
            name: reference.resolver().to_owned(),
        }
    })?;

    if !resolver.availability().allows(role) {
        return Err(InterpolationError::ResolverUnavailable {
            resolver: reference.resolver().to_owned(),
        });
    }
    if !resolver.validate_payload(reference.payload()) {
        return Err(InterpolationError::InvalidResolverPayload {
            resolver: reference.resolver().to_owned(),
            payload: reference.payload().to_owned(),
        });
    }

    Ok(resolver)
}

fn validate_variable<T: SchemaTypeMarker>(
    reference: &UntypedVariableReference,
    role: TemplateRole,
) -> Result<TypedVariable<T>, InterpolationError> {
    let resolver = validate_resolver_reference(reference, role)?;
    validate_variable_type(reference, resolver)
}

fn validate_variable_type<T: SchemaTypeMarker>(
    reference: &UntypedVariableReference,
    resolver: &ResolverEntry,
) -> Result<TypedVariable<T>, InterpolationError> {
    let expected = T::schema_type();
    let actual = resolver.output_type();
    if actual != &expected {
        return Err(InterpolationError::ResolverTypeMismatch {
            resolver: reference.resolver().to_owned(),
            expected,
            actual: actual.clone(),
        });
    }

    Ok(TypedVariable::validated(reference.clone()))
}

fn promote_parse_error(error: &ExpressionParseError) -> InterpolationError {
    match error {
        ExpressionParseError::UnclosedResolver { offset } => {
            InterpolationError::UnclosedResolver { offset: *offset }
        }
        ExpressionParseError::MissingPayloadSeparator { offset } => {
            InterpolationError::MissingPayloadSeparator { offset: *offset }
        }
        ExpressionParseError::NestedResolver { offset } => {
            InterpolationError::NestedResolver { offset: *offset }
        }
    }
}

pub fn promote_literal_string(
    source: &LiteralStringSource,
) -> Result<LiteralString, InterpolationError> {
    match source.parsed() {
        ParsedStringForm::Literal(literal) => {
            Ok(LiteralString::validated(literal.value().to_owned()))
        }
        ParsedStringForm::Variable(reference) => Err(InterpolationError::ResolverInLiteralString {
            resolver: reference.resolver().to_owned(),
        }),
        ParsedStringForm::Template(template) => {
            let resolver = template.parts().iter().find_map(|part| match part {
                ParsedTemplatePart::Literal(_) => None,
                ParsedTemplatePart::Variable(reference) => Some(reference.resolver()),
            });
            Err(InterpolationError::ResolverInLiteralString {
                resolver: resolver
                    .expect("a parsed template contains at least one variable")
                    .to_owned(),
            })
        }
        ParsedStringForm::Malformed(error) => Err(promote_parse_error(error)),
    }
}

pub fn promote_string_expression(
    source: &StringExpressionSource,
) -> Result<StringExpression, InterpolationError> {
    promote_string_form(source.parsed(), TemplateRole::Scalar)
}

fn promote_string_form(
    parsed: &ParsedStringForm,
    role: TemplateRole,
) -> Result<StringExpression, InterpolationError> {
    match parsed {
        ParsedStringForm::Literal(literal) => Ok(StringExpression::Literal(
            LiteralString::validated(literal.value().to_owned()),
        )),
        ParsedStringForm::Variable(reference) => {
            validate_variable(reference, role).map(StringExpression::Variable)
        }
        ParsedStringForm::Template(template) => {
            promote_string_template(template, role).map(StringExpression::Template)
        }
        ParsedStringForm::Malformed(error) => Err(promote_parse_error(error)),
    }
}

fn promote_string_template(
    template: &ParsedStringTemplate,
    role: TemplateRole,
) -> Result<StringTemplate<TypedVariable<StringType>>, InterpolationError> {
    let parts = template
        .parts()
        .iter()
        .map(|part| match part {
            ParsedTemplatePart::Literal(value) => Ok(StringTemplatePart::Literal(value.to_owned())),
            ParsedTemplatePart::Variable(reference) => {
                validate_string_template_variable(reference, role).map(StringTemplatePart::Variable)
            }
        })
        .collect::<Result<_, _>>()?;
    Ok(StringTemplate::validated(parts))
}

fn validate_string_template_variable(
    reference: &UntypedVariableReference,
    role: TemplateRole,
) -> Result<TypedVariable<StringType>, InterpolationError> {
    let resolver = validate_resolver_reference(reference, role)?;
    if role == TemplateRole::ProviderInstallArg
        && matches!(resolver.output_type(), SchemaType::List(_))
    {
        return Err(InterpolationError::ListResolverMustOccupyArgument {
            resolver: reference.resolver().to_owned(),
        });
    }
    validate_variable_type(reference, resolver)
}

pub fn promote_provider_install_arg(
    source: &ProviderInstallArgSource,
) -> Result<FlatListPart<StringType, StringExpression>, InterpolationError> {
    let ParsedStringForm::Variable(reference) = source.parsed() else {
        return promote_string_form(source.parsed(), TemplateRole::ProviderInstallArg)
            .map(FlatListPart::One);
    };

    let resolver = validate_resolver_reference(reference, TemplateRole::ProviderInstallArg)?;
    if resolver.output_type() == &ListType::<StringType>::schema_type() {
        validate_variable_type(reference, resolver).map(FlatListPart::Many)
    } else {
        validate_variable_type(reference, resolver)
            .map(StringExpression::Variable)
            .map(FlatListPart::One)
    }
}

pub fn promote_provider_install_args(
    sources: &[ProviderInstallArgSource],
) -> Result<ProviderInstallArgs, InterpolationError> {
    let parts = sources
        .iter()
        .map(promote_provider_install_arg)
        .collect::<Result<_, _>>()?;
    Ok(ProviderInstallArgs::validated(parts))
}

pub(crate) fn provider_args_resolver_count(expression: &ProviderInstallArgs) -> usize {
    expression
        .parts()
        .iter()
        .filter(|part| {
            matches!(
                part,
                FlatListPart::Many(variable)
                    if variable.reference().resolver() == "package"
                        && variable.reference().payload() == "provider_args"
            )
        })
        .count()
}

pub fn resolve_literal_string(
    source: &LiteralStringSource,
) -> Result<ResolvedString, InterpolationError> {
    let literal = promote_literal_string(source)?;
    Ok(ResolvedString::from(literal.value()))
}

pub fn resolve_string_expression(
    source: &StringExpressionSource,
    context: &ResolveContext<'_>,
) -> Result<ResolvedString, InterpolationError> {
    let expression = promote_string_expression(source)?;
    evaluate_string_expression(&expression, context)
}

pub fn resolve_environment_patch(
    patch: &EnvironmentPatch<StringExpressionSource>,
    context: &ResolveContext<'_>,
) -> Result<ResolvedEnvironmentPatch, InterpolationError> {
    Ok(ResolvedEnvironmentPatch {
        path_prepend: patch
            .path_prepend
            .as_ref()
            .map(|values| resolve_string_values(values, context))
            .transpose()?,
        path_append: patch
            .path_append
            .as_ref()
            .map(|values| resolve_string_values(values, context))
            .transpose()?,
        variables: patch
            .variables
            .iter()
            .map(|(name, value)| {
                resolve_string_expression(value, context).map(|value| (name.clone(), value))
            })
            .collect::<Result<_, _>>()?,
    })
}

pub fn resolve_exec_action(
    action: &SourceExecAction,
    context: &ResolveContext<'_>,
) -> Result<ResolvedExecAction, InterpolationError> {
    resolve_exec_action_with_fields(action, context).map_err(ExecActionResolutionError::into_source)
}

#[derive(Debug)]
pub(crate) enum ExecActionResolutionError {
    Program(InterpolationError),
    Argument {
        #[cfg_attr(not(feature = "native"), allow(dead_code))]
        index: usize,
        source: InterpolationError,
    },
    Cwd(InterpolationError),
    Env(InterpolationError),
}

impl ExecActionResolutionError {
    fn into_source(self) -> InterpolationError {
        match self {
            Self::Program(source)
            | Self::Argument { source, .. }
            | Self::Cwd(source)
            | Self::Env(source) => source,
        }
    }
}

pub(crate) fn resolve_exec_action_with_fields(
    action: &SourceExecAction,
    context: &ResolveContext<'_>,
) -> Result<ResolvedExecAction, ExecActionResolutionError> {
    Ok(ResolvedExecAction {
        program: resolve_string_expression(&action.program, context)
            .map_err(ExecActionResolutionError::Program)?,
        args: action
            .args
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                resolve_string_expression(argument, context)
                    .map_err(|source| ExecActionResolutionError::Argument { index, source })
            })
            .collect::<Result<_, _>>()?,
        cwd: action
            .cwd
            .as_ref()
            .map(|cwd| resolve_string_expression(cwd, context))
            .transpose()
            .map_err(ExecActionResolutionError::Cwd)?,
        env: action
            .env
            .as_ref()
            .map(|patch| resolve_environment_patch(patch, context))
            .transpose()
            .map_err(ExecActionResolutionError::Env)?,
    })
}

pub fn resolve_provider_install_action(
    action: &ExecAction<StringExpressionSource, ProviderInstallArgSource>,
    context: &ResolveContext<'_>,
) -> Result<ResolvedExecAction, InterpolationError> {
    let args = promote_provider_install_args(&action.args)?;
    resolve_provider_install_action_with_args(action, &args, context)
        .map_err(ExecActionResolutionError::into_source)
}

pub(crate) fn resolve_provider_install_action_with_args(
    action: &ExecAction<StringExpressionSource, ProviderInstallArgSource>,
    args: &ProviderInstallArgs,
    context: &ResolveContext<'_>,
) -> Result<ResolvedExecAction, ExecActionResolutionError> {
    let args = evaluate_provider_install_args(args, context)?;

    Ok(ResolvedExecAction {
        program: resolve_string_expression(&action.program, context)
            .map_err(ExecActionResolutionError::Program)?,
        args,
        cwd: action
            .cwd
            .as_ref()
            .map(|cwd| resolve_string_expression(cwd, context))
            .transpose()
            .map_err(ExecActionResolutionError::Cwd)?,
        env: action
            .env
            .as_ref()
            .map(|patch| resolve_environment_patch(patch, context))
            .transpose()
            .map_err(ExecActionResolutionError::Env)?,
    })
}

fn resolve_string_values(
    values: &OneOrMany<StringExpressionSource>,
    context: &ResolveContext<'_>,
) -> Result<OneOrMany<ResolvedString>, InterpolationError> {
    match values {
        OneOrMany::One(value) => resolve_string_expression(value, context).map(OneOrMany::One),
        OneOrMany::Many(values) => values
            .iter()
            .map(|value| resolve_string_expression(value, context))
            .collect::<Result<_, _>>()
            .map(OneOrMany::Many),
    }
}

fn evaluate_provider_install_args(
    expression: &ProviderInstallArgs,
    context: &ResolveContext<'_>,
) -> Result<Vec<ResolvedString>, ExecActionResolutionError> {
    let mut values = Vec::new();
    for (index, part) in expression.parts().iter().enumerate() {
        match part {
            FlatListPart::One(expression) => {
                values.push(
                    evaluate_string_expression(expression, context)
                        .map_err(|source| ExecActionResolutionError::Argument { index, source })?,
                );
            }
            FlatListPart::Many(variable) => {
                values.extend(
                    evaluate_string_list_variable(variable, context)
                        .map_err(|source| ExecActionResolutionError::Argument { index, source })?,
                );
            }
        }
    }
    Ok(values)
}

fn evaluate_string_expression(
    expression: &StringExpression,
    context: &ResolveContext<'_>,
) -> Result<ResolvedString, InterpolationError> {
    match expression {
        StringExpression::Literal(value) => Ok(ResolvedString::from(value.value())),
        StringExpression::Variable(variable) => evaluate_string_variable(variable, context),
        StringExpression::Template(template) => {
            let mut result = String::new();
            for part in template.parts() {
                match part {
                    StringTemplatePart::Literal(value) => result.push_str(value),
                    StringTemplatePart::Variable(variable) => {
                        result.push_str(evaluate_string_variable(variable, context)?.value());
                    }
                }
            }
            Ok(ResolvedString::from(result))
        }
    }
}

fn evaluate_string_variable(
    variable: &TypedVariable<StringType>,
    context: &ResolveContext<'_>,
) -> Result<ResolvedString, InterpolationError> {
    let reference = variable.reference();
    resolve_variable(reference.resolver(), reference.payload(), context)?
        .into_string(reference.resolver())
}

fn evaluate_string_list_variable(
    variable: &TypedVariable<ListType<StringType>>,
    context: &ResolveContext<'_>,
) -> Result<Vec<ResolvedString>, InterpolationError> {
    let reference = variable.reference();
    resolve_variable(reference.resolver(), reference.payload(), context)?
        .into_string_list(reference.resolver())
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InterpolationError {
    #[error("unclosed resolver call at byte {offset}")]
    UnclosedResolver { offset: usize },
    #[error("resolver call at byte {offset} is missing the `:` payload separator")]
    MissingPayloadSeparator { offset: usize },
    #[error("nested resolver call at byte {offset}")]
    NestedResolver { offset: usize },
    #[error("unknown resolver `{name}`")]
    UnknownResolver { name: String },
    #[error("invalid payload `{payload}` for resolver `{resolver}`")]
    InvalidResolverPayload { resolver: String, payload: String },
    #[error("resolver `{resolver}` is unavailable in this context")]
    ResolverUnavailable { resolver: String },
    #[error("resolver `{resolver}` has type {actual:?}, but this context requires {expected:?}")]
    ResolverTypeMismatch {
        resolver: String,
        expected: SchemaType,
        actual: SchemaType,
    },
    #[error("resolver `{resolver}` returned {actual:?}, but declared {expected:?}")]
    ResolverContractViolation {
        resolver: String,
        expected: SchemaType,
        actual: SchemaType,
    },
    #[error("resolver `{resolver}` is not allowed in a literal string")]
    ResolverInLiteralString { resolver: String },
    #[error("list resolver `{resolver}` must occupy one complete argument")]
    ListResolverMustOccupyArgument { resolver: String },
    #[error("environment variable `{name}` is not defined")]
    MissingEnvironmentVariable { name: String },
    #[error("environment variable `{name}` is not Unicode")]
    NonUnicodeEnvironmentVariable { name: String },
    #[error("path value `{name}` is unavailable")]
    UnavailablePath { name: String },
    #[error("path value `{name}` is not Unicode")]
    NonUnicodePath { name: String },
    #[error("package resolver requires a provider package batch")]
    MissingPackageContext,
}
