mod resolver;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use directories::{BaseDirs, UserDirs};

use crate::action::ExecutionEnvironment;
use crate::schema::{
    EnvironmentPatch, ExecAction, ExpressionParseError, FlatListPart, ListType, LiteralString,
    LiteralStringSource, OneOrMany, ParsedStringForm, ParsedTemplate as ParsedStringTemplate,
    ParsedTemplatePart, ProviderInstallArgSource, ProviderInstallArgs, ResolvedEnvironmentPatch,
    ResolvedExecAction, ResolvedString, SchemaType, SchemaTypeMarker, SourceExecAction,
    StringExpression, StringExpressionSource, StringTemplate, StringTemplatePart, StringType,
    TypedVariable, UntypedVariableReference,
};

use resolver::{ResolvedValue, ResolverEntry, lookup_resolver};

#[derive(Clone, Copy, Debug)]
pub struct DotPaths<'a> {
    config: &'a Path,
    config_dir: &'a Path,
    cwd: &'a Path,
}

impl<'a> DotPaths<'a> {
    pub const fn new(config: &'a Path, config_dir: &'a Path, cwd: &'a Path) -> Self {
        Self {
            config,
            config_dir,
            cwd,
        }
    }

    pub const fn config_dir(&self) -> &'a Path {
        self.config_dir
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DotPath {
    Config,
    ConfigDir,
    Cwd,
}

impl DotPath {
    fn from_payload(payload: &str) -> Option<Self> {
        match payload {
            "config" => Some(Self::Config),
            "config_dir" => Some(Self::ConfigDir),
            "cwd" => Some(Self::Cwd),
            _ => None,
        }
    }
}

impl DotPaths<'_> {
    fn get(&self, path: DotPath) -> &Path {
        match path {
            DotPath::Config => self.config,
            DotPath::ConfigDir => self.config_dir,
            DotPath::Cwd => self.cwd,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum XdgPath {
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
    values: BTreeMap<XdgPath, PathBuf>,
}

impl XdgPaths {
    pub fn detect() -> Self {
        let mut values = BTreeMap::new();

        if let Some(base) = BaseDirs::new() {
            values.insert(XdgPath::Home, base.home_dir().to_path_buf());
            values.insert(XdgPath::Config, base.config_dir().to_path_buf());
            values.insert(XdgPath::ConfigLocal, base.config_local_dir().to_path_buf());
            values.insert(XdgPath::Data, base.data_dir().to_path_buf());
            values.insert(XdgPath::DataLocal, base.data_local_dir().to_path_buf());
            values.insert(XdgPath::Cache, base.cache_dir().to_path_buf());
            if let Some(path) = base.state_dir() {
                values.insert(XdgPath::State, path.to_path_buf());
            }
            if let Some(path) = base.runtime_dir() {
                values.insert(XdgPath::Runtime, path.to_path_buf());
            }
            if let Some(path) = base.executable_dir() {
                values.insert(XdgPath::Executable, path.to_path_buf());
            }
        }

        if let Some(user) = UserDirs::new()
            && let Some(path) = user.document_dir()
        {
            values.insert(XdgPath::Documents, path.to_path_buf());
        }

        Self { values }
    }

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
    Ok(ResolvedExecAction {
        kind: action.kind,
        program: resolve_string_expression(&action.program, context)?,
        args: action
            .args
            .iter()
            .map(|argument| resolve_string_expression(argument, context))
            .collect::<Result<_, _>>()?,
        cwd: action
            .cwd
            .as_ref()
            .map(|cwd| resolve_string_expression(cwd, context))
            .transpose()?,
        env: action
            .env
            .as_ref()
            .map(|patch| resolve_environment_patch(patch, context))
            .transpose()?,
    })
}

pub fn resolve_provider_install_action(
    action: &ExecAction<StringExpressionSource, ProviderInstallArgSource>,
    context: &ResolveContext<'_>,
) -> Result<ResolvedExecAction, InterpolationError> {
    let args = promote_provider_install_args(&action.args)?;
    resolve_provider_install_action_with_args(action, &args, context)
}

pub(crate) fn resolve_provider_install_action_with_args(
    action: &ExecAction<StringExpressionSource, ProviderInstallArgSource>,
    args: &ProviderInstallArgs,
    context: &ResolveContext<'_>,
) -> Result<ResolvedExecAction, InterpolationError> {
    let args = evaluate_provider_install_args(args, context)?;

    Ok(ResolvedExecAction {
        kind: action.kind,
        program: resolve_string_expression(&action.program, context)?,
        args,
        cwd: action
            .cwd
            .as_ref()
            .map(|cwd| resolve_string_expression(cwd, context))
            .transpose()?,
        env: action
            .env
            .as_ref()
            .map(|patch| resolve_environment_patch(patch, context))
            .transpose()?,
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
) -> Result<Vec<ResolvedString>, InterpolationError> {
    let mut values = Vec::new();
    for part in expression.parts() {
        match part {
            FlatListPart::One(expression) => {
                values.push(evaluate_string_expression(expression, context)?);
            }
            FlatListPart::Many(variable) => {
                values.extend(evaluate_string_list_variable(variable, context)?);
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
    let resolver = lookup_resolver(reference.resolver()).expect("typed resolver exists");
    let ResolvedValue::String(value) = resolver.resolve(reference.payload(), context)? else {
        unreachable!("typed string resolver produces a scalar");
    };
    Ok(value)
}

fn evaluate_string_list_variable(
    variable: &TypedVariable<ListType<StringType>>,
    context: &ResolveContext<'_>,
) -> Result<Vec<ResolvedString>, InterpolationError> {
    let reference = variable.reference();
    let resolver = lookup_resolver(reference.resolver()).expect("typed resolver exists");
    let ResolvedValue::StringList(values) = resolver.resolve(reference.payload(), context)? else {
        unreachable!("typed string-list resolver produces a list");
    };
    Ok(values)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InterpolationError {
    UnclosedResolver {
        offset: usize,
    },
    MissingPayloadSeparator {
        offset: usize,
    },
    NestedResolver {
        offset: usize,
    },
    UnknownResolver {
        name: String,
    },
    InvalidResolverPayload {
        resolver: String,
        payload: String,
    },
    ResolverUnavailable {
        resolver: String,
    },
    ResolverTypeMismatch {
        resolver: String,
        expected: SchemaType,
        actual: SchemaType,
    },
    ResolverInLiteralString {
        resolver: String,
    },
    ListResolverMustOccupyArgument {
        resolver: String,
    },
    MissingEnvironmentVariable {
        name: String,
    },
    NonUnicodeEnvironmentVariable {
        name: String,
    },
    UnavailablePath {
        name: String,
    },
    NonUnicodePath {
        name: String,
    },
    MissingPackageContext,
}

impl fmt::Display for InterpolationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnclosedResolver { offset } => {
                write!(formatter, "unclosed resolver call at byte {offset}")
            }
            Self::MissingPayloadSeparator { offset } => write!(
                formatter,
                "resolver call at byte {offset} is missing the `:` payload separator"
            ),
            Self::NestedResolver { offset } => {
                write!(formatter, "nested resolver call at byte {offset}")
            }
            Self::UnknownResolver { name } => write!(formatter, "unknown resolver `{name}`"),
            Self::InvalidResolverPayload { resolver, payload } => {
                write!(
                    formatter,
                    "invalid payload `{payload}` for resolver `{resolver}`"
                )
            }
            Self::ResolverUnavailable { resolver } => {
                write!(
                    formatter,
                    "resolver `{resolver}` is unavailable in this context"
                )
            }
            Self::ResolverTypeMismatch {
                resolver,
                expected,
                actual,
            } => write!(
                formatter,
                "resolver `{resolver}` has type {actual:?}, but this context requires {expected:?}"
            ),
            Self::ResolverInLiteralString { resolver } => write!(
                formatter,
                "resolver `{resolver}` is not allowed in a literal string"
            ),
            Self::ListResolverMustOccupyArgument { resolver } => write!(
                formatter,
                "list resolver `{resolver}` must occupy one complete argument"
            ),
            Self::MissingEnvironmentVariable { name } => {
                write!(formatter, "environment variable `{name}` is not defined")
            }
            Self::NonUnicodeEnvironmentVariable { name } => {
                write!(formatter, "environment variable `{name}` is not Unicode")
            }
            Self::UnavailablePath { name } => {
                write!(formatter, "path value `{name}` is unavailable")
            }
            Self::NonUnicodePath { name } => {
                write!(formatter, "path value `{name}` is not Unicode")
            }
            Self::MissingPackageContext => {
                formatter.write_str("package resolver requires a provider package batch")
            }
        }
    }
}

impl Error for InterpolationError {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use directories::{BaseDirs, UserDirs};

    use crate::action::ExecutionEnvironment;
    use crate::schema::{
        EnvironmentName, EnvironmentPatch, ExecAction, FlatListPart, ListType, LiteralStringSource,
        OneOrMany, ParsedStringForm, ProviderInstallArgSource, ResolvedEnvironmentPatch,
        ResolvedString, StringExpression, StringExpressionSource, StringTemplatePart, StringType,
        TypedVariable,
    };

    use super::{
        DotPaths, InterpolationError, PackageContext, ResolveContext, XdgPath, XdgPaths,
        evaluate_provider_install_args, promote_literal_string, promote_provider_install_arg,
        promote_provider_install_args, promote_string_expression, resolve_environment_patch,
        resolve_exec_action, resolve_literal_string, resolve_provider_install_action,
        resolve_string_expression,
    };

    fn environment(variables: &[(&str, &str)]) -> ExecutionEnvironment {
        let patch = ResolvedEnvironmentPatch {
            path_prepend: None,
            path_append: None,
            variables: variables
                .iter()
                .map(|(name, value)| {
                    (
                        EnvironmentName::new(*name).expect("test name should be valid"),
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

    fn dot_paths() -> DotPaths<'static> {
        DotPaths::new(
            Path::new("/repo/dot.toml"),
            Path::new("/repo"),
            Path::new("/work"),
        )
    }

    fn xdg_paths(entries: &[(XdgPath, &str)]) -> XdgPaths {
        XdgPaths {
            values: entries
                .iter()
                .map(|(key, value)| (*key, PathBuf::from(value)))
                .collect(),
        }
    }

    #[test]
    fn promotes_an_exact_string_resolver_to_a_variable_node() {
        let promoted = promote_string_expression(&StringExpressionSource::from("${env:HOME}"))
            .expect("the environment resolver produces a string");

        let StringExpression::Variable(variable) = promoted else {
            panic!("an exact variable must retain its syntax node");
        };
        assert_eq!(variable.reference().resolver(), "env");
        assert_eq!(variable.reference().payload(), "HOME");
    }

    #[test]
    fn promotes_an_exact_list_resolver_to_a_many_part() {
        let promoted =
            promote_provider_install_arg(&ProviderInstallArgSource::from("${package:names}"))
                .expect("package names produce a string list");

        let FlatListPart::Many(variable) = promoted else {
            panic!("an exact list variable must expand as a many part");
        };
        assert_eq!(variable.reference().resolver(), "package");
        assert_eq!(variable.reference().payload(), "names");
    }

    #[test]
    fn promotes_literal_and_template_provider_args_to_one_parts() {
        let literal = promote_provider_install_arg(&ProviderInstallArgSource::from("install"))
            .expect("a literal provider argument is one string");
        assert!(matches!(
            literal,
            FlatListPart::One(StringExpression::Literal(value)) if value.value() == "install"
        ));

        let template =
            promote_provider_install_arg(&ProviderInstallArgSource::from("--root=${env:HOME}"))
                .expect("a string template provider argument is one string");
        let FlatListPart::One(StringExpression::Template(template)) = template else {
            panic!("a template provider argument must remain a single string expression");
        };
        assert_eq!(template.parts().len(), 2);
    }

    #[test]
    fn rejects_a_list_variable_inside_a_string_template() {
        let error = promote_provider_install_arg(&ProviderInstallArgSource::from(
            "prefix-${package:names}",
        ))
        .expect_err("a list variable cannot be embedded in one string");

        assert_eq!(
            error,
            InterpolationError::ListResolverMustOccupyArgument {
                resolver: "package".into(),
            }
        );
    }

    #[test]
    fn reports_stored_syntax_errors_only_during_promotion() {
        let source = StringExpressionSource::from("prefix-${env:HOME");
        assert!(matches!(source.parsed(), ParsedStringForm::Malformed(_)));

        assert_eq!(
            promote_string_expression(&source),
            Err(InterpolationError::UnclosedResolver { offset: 7 })
        );
    }

    #[test]
    fn reports_unknown_resolvers_during_promotion() {
        assert_eq!(
            promote_string_expression(&StringExpressionSource::from("${future:value}")),
            Err(InterpolationError::UnknownResolver {
                name: "future".into(),
            })
        );
    }

    #[test]
    fn reports_invalid_resolver_payloads_during_promotion() {
        assert_eq!(
            promote_string_expression(&StringExpressionSource::from("${xdg:repository}")),
            Err(InterpolationError::InvalidResolverPayload {
                resolver: "xdg".into(),
                payload: "repository".into(),
            })
        );
    }

    #[test]
    fn package_resolver_is_unavailable_in_a_scalar_role() {
        assert_eq!(
            promote_string_expression(&StringExpressionSource::from("${package:names}")),
            Err(InterpolationError::ResolverUnavailable {
                resolver: "package".into(),
            })
        );
    }

    #[test]
    fn exact_scalar_and_list_variables_have_distinct_typed_nodes() {
        fn assert_string_variable(_: &TypedVariable<StringType>) {}
        fn assert_string_list_variable(_: &TypedVariable<ListType<StringType>>) {}

        let scalar = promote_provider_install_arg(&ProviderInstallArgSource::from("${env:HOME}"))
            .expect("an exact scalar resolver is one provider argument");
        let FlatListPart::One(StringExpression::Variable(variable)) = scalar else {
            panic!("the exact scalar variable must be a one part");
        };
        assert_string_variable(&variable);

        let list =
            promote_provider_install_arg(&ProviderInstallArgSource::from("${package:names}"))
                .expect("an exact list resolver is a many provider argument");
        let FlatListPart::Many(variable) = list else {
            panic!("the exact list variable must be a many part");
        };
        assert_string_list_variable(&variable);
    }

    #[test]
    fn string_templates_contain_only_string_typed_variables() {
        fn assert_string_variable(_: &TypedVariable<StringType>) {}

        let promoted =
            promote_string_expression(&StringExpressionSource::from("${env:HOME}/${dot:cwd}"))
                .expect("both variables produce strings");
        let StringExpression::Template(template) = promoted else {
            panic!("literal surroundings and multiple variables form a template");
        };

        for part in template.parts() {
            if let StringTemplatePart::Variable(variable) = part {
                assert_string_variable(variable);
            }
        }
    }

    #[test]
    fn promotes_validated_unescaped_literal_values() {
        let promoted = promote_literal_string(&LiteralStringSource::from(r"prefix-\${literal}"))
            .expect("escaped resolver syntax is literal");

        assert_eq!(promoted.value(), "prefix-${literal}");
    }

    #[test]
    fn literal_source_promotion_rejects_resolvers() {
        assert_eq!(
            promote_literal_string(&LiteralStringSource::from("${env:HOME}")),
            Err(InterpolationError::ResolverInLiteralString {
                resolver: "env".into(),
            })
        );
    }

    #[test]
    fn promotes_provider_arg_vectors_into_one_flat_list() {
        let sources = [
            ProviderInstallArgSource::from("install"),
            ProviderInstallArgSource::from("${package:names}"),
        ];
        let promoted =
            promote_provider_install_args(&sources).expect("both provider arguments are valid");

        assert!(matches!(
            promoted.parts(),
            [FlatListPart::One(_), FlatListPart::Many(_)]
        ));
    }

    #[test]
    fn literal_strings_unescape_syntax_but_reject_resolvers() {
        assert_eq!(
            resolve_literal_string(&LiteralStringSource::from(r"prefix-\${literal}"))
                .expect("escaped interpolation syntax should be literal")
                .value(),
            "prefix-${literal}",
        );
        assert!(
            resolve_literal_string(&LiteralStringSource::from("${env:HOME}"))
                .expect_err("literal strings must reject resolvers")
                .to_string()
                .contains("literal string")
        );
    }

    #[test]
    fn dot_resolver_produces_invocation_paths() {
        let environment = environment(&[]);
        let xdg = xdg_paths(&[]);
        let context = ResolveContext::new(&environment, dot_paths(), &xdg);
        let template = StringExpressionSource::from("${dot:config}:${dot:config_dir}:${dot:cwd}");

        assert_eq!(
            resolve_string_expression(&template, &context)
                .expect("template should resolve")
                .value(),
            "/repo/dot.toml:/repo:/work"
        );
    }

    #[test]
    fn environment_patch_resolves_every_value_against_one_context() {
        let environment = environment(&[("ROOT", "/opt/tools")]);
        let xdg = xdg_paths(&[(XdgPath::Executable, "/home/tester/.local/bin")]);
        let context = ResolveContext::new(&environment, dot_paths(), &xdg);
        let patch = EnvironmentPatch {
            path_prepend: Some(OneOrMany::Many(vec![
                "${env:ROOT}/bin".into(),
                "${dot:config_dir}/bin".into(),
            ])),
            path_append: Some(OneOrMany::One("${xdg:executable}".into())),
            variables: BTreeMap::from([(
                EnvironmentName::new("TOOL_HOME").expect("test name should be valid"),
                "${env:ROOT}".into(),
            )]),
        };

        let resolved = resolve_environment_patch(&patch, &context).expect("patch should resolve");

        assert_eq!(
            resolved.path_prepend,
            Some(OneOrMany::Many(vec![
                "/opt/tools/bin".into(),
                "/repo/bin".into(),
            ]))
        );
        assert_eq!(
            resolved.path_append,
            Some(OneOrMany::One("/home/tester/.local/bin".into()))
        );
        assert_eq!(resolved.variables["TOOL_HOME"].value(), "/opt/tools");
    }

    #[test]
    fn exec_action_resolves_all_process_fields_and_its_environment() {
        let environment = environment(&[("PROBE", "probe-program"), ("ROOT", "/opt/tools")]);
        let xdg = xdg_paths(&[(XdgPath::Documents, "/home/tester/Documents")]);
        let context = ResolveContext::new(&environment, dot_paths(), &xdg);
        let action = ExecAction {
            kind: None,
            program: "${env:PROBE}".into(),
            args: vec!["--config=${dot:config}".into(), "${xdg:documents}".into()],
            cwd: Some("${dot:cwd}".into()),
            env: Some(EnvironmentPatch {
                path_prepend: None,
                path_append: None,
                variables: BTreeMap::from([(
                    EnvironmentName::new("TOOL_HOME").expect("test name should be valid"),
                    "${env:ROOT}".into(),
                )]),
            }),
        };

        let resolved = resolve_exec_action(&action, &context).expect("action should resolve");

        assert_eq!(resolved.program.value(), "probe-program");
        assert_eq!(
            resolved
                .args
                .iter()
                .map(ResolvedString::value)
                .collect::<Vec<_>>(),
            vec!["--config=/repo/dot.toml", "/home/tester/Documents"]
        );
        assert_eq!(resolved.cwd.as_ref().unwrap().value(), "/work");
        assert_eq!(
            resolved.env.as_ref().unwrap().variables["TOOL_HOME"].value(),
            "/opt/tools"
        );
    }

    #[test]
    fn xdg_resolver_produces_cross_platform_standard_paths() {
        let environment = environment(&[]);
        let xdg = xdg_paths(&[
            (XdgPath::Home, "/home/tester"),
            (XdgPath::Config, "/home/tester/.config"),
            (XdgPath::Documents, "/home/tester/Documents"),
        ]);
        let context = ResolveContext::new(&environment, dot_paths(), &xdg);
        let template = StringExpressionSource::from("${xdg:home}:${xdg:config}:${xdg:documents}");

        assert_eq!(
            resolve_string_expression(&template, &context)
                .expect("template should resolve")
                .value(),
            "/home/tester:/home/tester/.config:/home/tester/Documents"
        );
    }

    #[test]
    fn xdg_detection_snapshots_the_platform_directories() {
        let detected = XdgPaths::detect();
        let base = BaseDirs::new().expect("the test process should have a home directory");

        assert_eq!(detected.get(XdgPath::Home), Some(base.home_dir()));
        assert_eq!(detected.get(XdgPath::Config), Some(base.config_dir()));
        assert_eq!(detected.get(XdgPath::Data), Some(base.data_dir()));
        assert_eq!(detected.get(XdgPath::Cache), Some(base.cache_dir()));

        let expected_documents = UserDirs::new()
            .and_then(|directories| directories.document_dir().map(Path::to_path_buf));
        assert_eq!(
            detected.get(XdgPath::Documents),
            expected_documents.as_deref()
        );
    }

    #[test]
    fn every_declared_xdg_payload_is_valid() {
        for payload in [
            "home",
            "config",
            "config_local",
            "data",
            "data_local",
            "cache",
            "state",
            "runtime",
            "executable",
            "documents",
        ] {
            let template = StringExpressionSource::from(format!("${{xdg:{payload}}}"));
            promote_string_expression(&template)
                .unwrap_or_else(|error| panic!("xdg payload `{payload}` should be valid: {error}"));
        }
    }

    #[test]
    fn old_path_resolver_is_not_registered() {
        let template = StringExpressionSource::from("${path:cwd}");

        assert_eq!(
            promote_string_expression(&template),
            Err(InterpolationError::UnknownResolver {
                name: "path".into()
            })
        );
    }

    #[test]
    fn validation_rejects_unknown_resolvers_without_evaluating_them() {
        let template = StringExpressionSource::from("${command:output}");

        assert_eq!(
            promote_string_expression(&template),
            Err(InterpolationError::UnknownResolver {
                name: "command".into()
            })
        );
    }

    #[test]
    fn resolver_definitions_validate_their_own_payloads() {
        let template = StringExpressionSource::from("${xdg:repository}");

        assert_eq!(
            promote_string_expression(&template),
            Err(InterpolationError::InvalidResolverPayload {
                resolver: "xdg".into(),
                payload: "repository".into(),
            })
        );
    }

    #[test]
    fn package_resolvers_are_available_only_to_provider_install_arguments() {
        let scalar = StringExpressionSource::from("${package:names}");
        let install_arg = ProviderInstallArgSource::from("${package:names}");

        assert_eq!(
            promote_string_expression(&scalar),
            Err(InterpolationError::ResolverUnavailable {
                resolver: "package".into()
            })
        );
        promote_provider_install_arg(&install_arg)
            .expect("package resolver should be valid for provider install");
    }

    #[test]
    fn provider_install_list_resolvers_expand_one_complete_argument() {
        let environment = environment(&[]);
        let xdg = xdg_paths(&[]);
        let names = ["ripgrep".into(), "zoxide".into()];
        let provider_args = ["--locked".into()];
        let packages = PackageContext::new(&names, &provider_args);
        let context = ResolveContext::new(&environment, dot_paths(), &xdg).with_package(packages);

        let expression = promote_provider_install_args(&[
            ProviderInstallArgSource::from("${package:names}"),
            ProviderInstallArgSource::from("${package:provider_args}"),
        ])
        .expect("provider arguments should promote");
        let resolved = evaluate_provider_install_args(&expression, &context)
            .expect("package values should resolve");

        assert_eq!(
            resolved
                .iter()
                .map(ResolvedString::value)
                .collect::<Vec<_>>(),
            vec!["ripgrep", "zoxide", "--locked"]
        );
    }

    #[test]
    fn provider_install_action_expands_package_lists_into_argv() {
        let environment = environment(&[("PROVIDER", "brew")]);
        let xdg = xdg_paths(&[]);
        let names = ["font-one".into(), "font-two".into()];
        let provider_args = ["--cask".into(), "--force".into()];
        let context = ResolveContext::new(&environment, dot_paths(), &xdg)
            .with_package(PackageContext::new(&names, &provider_args));
        let action = ExecAction::<StringExpressionSource, ProviderInstallArgSource> {
            kind: None,
            program: "${env:PROVIDER}".into(),
            args: vec![
                "install".into(),
                "${package:provider_args}".into(),
                "--config=${dot:config}".into(),
                "${package:names}".into(),
            ],
            cwd: Some("${dot:cwd}".into()),
            env: None,
        };

        let resolved =
            resolve_provider_install_action(&action, &context).expect("action should resolve");

        assert_eq!(resolved.program.value(), "brew");
        assert_eq!(
            resolved
                .args
                .iter()
                .map(ResolvedString::value)
                .collect::<Vec<_>>(),
            vec![
                "install",
                "--cask",
                "--force",
                "--config=/repo/dot.toml",
                "font-one",
                "font-two",
            ]
        );
        assert_eq!(resolved.cwd.as_ref().unwrap().value(), "/work");
    }

    #[test]
    fn provider_install_action_reports_an_embedded_list_resolver() {
        let environment = environment(&[]);
        let xdg = xdg_paths(&[]);
        let names = ["ripgrep".into()];
        let provider_args = Vec::new();
        let context = ResolveContext::new(&environment, dot_paths(), &xdg)
            .with_package(PackageContext::new(&names, &provider_args));
        let action = ExecAction::<StringExpressionSource, ProviderInstallArgSource> {
            kind: None,
            program: "install".into(),
            args: vec!["prefix-${package:names}".into()],
            cwd: None,
            env: None,
        };

        assert_eq!(
            resolve_provider_install_action(&action, &context),
            Err(InterpolationError::ListResolverMustOccupyArgument {
                resolver: "package".into()
            })
        );
    }
}
