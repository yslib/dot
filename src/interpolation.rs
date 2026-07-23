use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use directories::{BaseDirs, UserDirs};

use crate::action::ExecutionEnvironment;
use crate::schema::{
    EnvironmentName, EnvironmentPatch, ExecAction, ExpressionParseError, FlatListPart, ListType,
    LiteralString, LiteralStringSource, OneOrMany, ParsedStringForm,
    ParsedTemplate as ParsedStringTemplate, ParsedTemplatePart, ProviderInstallArg,
    ProviderInstallArgSource, ProviderInstallArgs, ScalarTemplate, SchemaType, SchemaTypeMarker,
    StringExpression, StringExpressionSource, StringTemplate, StringTemplatePart, StringType,
    TypedVariable, UntypedVariableReference, ValidatedLiteralString,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedTemplate {
    segments: Vec<TemplateSegment>,
}

impl ParsedTemplate {
    fn segments(&self) -> &[TemplateSegment] {
        &self.segments
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TemplateSegment {
    Literal(String),
    Resolver(ResolverCall),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolverCall {
    name: String,
    payload: String,
}

impl ResolverCall {
    fn new(name: impl Into<String>, payload: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            payload: payload.into(),
        }
    }
}

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
enum ResolverAvailability {
    Everywhere,
    ProviderInstallOnly,
}

impl ResolverAvailability {
    fn allows(self, role: TemplateRole) -> bool {
        self == Self::Everywhere || role == TemplateRole::ProviderInstallArg
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TemplateRole {
    Scalar,
    ProviderInstallArg,
}

enum ResolverValue {
    Scalar(String),
    List(Vec<String>),
}

type ValidatePayloadFn = fn(&str) -> bool;
type SchemaTypeFn = fn() -> SchemaType;
type ResolveFn = for<'a> fn(&str, &ResolveContext<'a>) -> Result<ResolverValue, InterpolationError>;

struct BuiltinResolverSpec {
    name: &'static str,
    output: SchemaTypeFn,
    availability: ResolverAvailability,
    validate_payload: ValidatePayloadFn,
    resolve: ResolveFn,
}

fn string_schema_type() -> SchemaType {
    StringType::schema_type()
}

fn string_list_schema_type() -> SchemaType {
    ListType::<StringType>::schema_type()
}

static BUILTIN_RESOLVERS: &[BuiltinResolverSpec] = &[
    BuiltinResolverSpec {
        name: "env",
        output: string_schema_type,
        availability: ResolverAvailability::Everywhere,
        validate_payload: validate_env_payload,
        resolve: resolve_env,
    },
    BuiltinResolverSpec {
        name: "dot",
        output: string_schema_type,
        availability: ResolverAvailability::Everywhere,
        validate_payload: validate_dot_payload,
        resolve: resolve_dot,
    },
    BuiltinResolverSpec {
        name: "xdg",
        output: string_schema_type,
        availability: ResolverAvailability::Everywhere,
        validate_payload: validate_xdg_payload,
        resolve: resolve_xdg,
    },
    BuiltinResolverSpec {
        name: "package",
        output: string_list_schema_type,
        availability: ResolverAvailability::ProviderInstallOnly,
        validate_payload: validate_package_payload,
        resolve: resolve_package,
    },
];

fn lookup_resolver(name: &str) -> Option<&'static BuiltinResolverSpec> {
    BUILTIN_RESOLVERS
        .iter()
        .find(|resolver| resolver.name == name)
}

fn validate_env_payload(payload: &str) -> bool {
    EnvironmentName::new(payload).is_ok()
}

fn validate_dot_payload(payload: &str) -> bool {
    DotPath::from_payload(payload).is_some()
}

fn validate_xdg_payload(payload: &str) -> bool {
    XdgPath::from_payload(payload).is_some()
}

fn validate_package_payload(payload: &str) -> bool {
    matches!(payload, "names" | "provider_args")
}

fn resolve_env(
    payload: &str,
    context: &ResolveContext<'_>,
) -> Result<ResolverValue, InterpolationError> {
    let value = context.environment.get(payload).ok_or_else(|| {
        InterpolationError::MissingEnvironmentVariable {
            name: payload.to_owned(),
        }
    })?;
    let value =
        value
            .to_str()
            .ok_or_else(|| InterpolationError::NonUnicodeEnvironmentVariable {
                name: payload.to_owned(),
            })?;
    Ok(ResolverValue::Scalar(value.to_owned()))
}

fn resolve_dot(
    payload: &str,
    context: &ResolveContext<'_>,
) -> Result<ResolverValue, InterpolationError> {
    let path = context.dot.get(
        DotPath::from_payload(payload).expect("payload was validated by the resolver definition"),
    );
    let value = path
        .to_str()
        .ok_or_else(|| InterpolationError::NonUnicodePath {
            name: payload.to_owned(),
        })?;
    Ok(ResolverValue::Scalar(value.to_owned()))
}

fn resolve_xdg(
    payload: &str,
    context: &ResolveContext<'_>,
) -> Result<ResolverValue, InterpolationError> {
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
    Ok(ResolverValue::Scalar(value.to_owned()))
}

fn resolve_package(
    payload: &str,
    context: &ResolveContext<'_>,
) -> Result<ResolverValue, InterpolationError> {
    let package = context
        .package
        .ok_or(InterpolationError::MissingPackageContext)?;
    let values = match payload {
        "names" => package.names,
        "provider_args" => package.provider_args,
        _ => &[],
    };
    Ok(ResolverValue::List(values.to_vec()))
}

fn validate_resolver_reference(
    reference: &UntypedVariableReference,
    role: TemplateRole,
) -> Result<&'static BuiltinResolverSpec, InterpolationError> {
    let resolver = lookup_resolver(reference.resolver()).ok_or_else(|| {
        InterpolationError::UnknownResolver {
            name: reference.resolver().to_owned(),
        }
    })?;

    if !resolver.availability.allows(role) {
        return Err(InterpolationError::ResolverUnavailable {
            resolver: reference.resolver().to_owned(),
        });
    }
    if !(resolver.validate_payload)(reference.payload()) {
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
    let expected = T::schema_type();
    let actual = (resolver.output)();
    if actual != expected {
        return Err(InterpolationError::ResolverTypeMismatch {
            resolver: reference.resolver().to_owned(),
            expected,
            actual,
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
) -> Result<ValidatedLiteralString, InterpolationError> {
    match source.parsed() {
        ParsedStringForm::Literal(literal) => Ok(ValidatedLiteralString::validated(
            literal.value().to_owned(),
        )),
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
            ValidatedLiteralString::validated(literal.value().to_owned()),
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
                validate_variable(reference, role).map(StringTemplatePart::Variable)
            }
        })
        .collect::<Result<_, _>>()?;
    Ok(StringTemplate::validated(parts))
}

pub fn promote_provider_install_arg(
    source: &ProviderInstallArgSource,
) -> Result<FlatListPart<StringType, StringExpression>, InterpolationError> {
    let ParsedStringForm::Variable(reference) = source.parsed() else {
        return promote_string_form(source.parsed(), TemplateRole::ProviderInstallArg)
            .map(FlatListPart::One);
    };

    let resolver = validate_resolver_reference(reference, TemplateRole::ProviderInstallArg)?;
    if (resolver.output)() == ListType::<StringType>::schema_type() {
        validate_variable(reference, TemplateRole::ProviderInstallArg).map(FlatListPart::Many)
    } else {
        validate_variable(reference, TemplateRole::ProviderInstallArg)
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

pub fn validate_scalar_template(template: &ScalarTemplate) -> Result<(), InterpolationError> {
    let parsed = parse_template(template.as_str())?;
    validate_template(&parsed, TemplateRole::Scalar)
}

pub fn resolve_literal_string(value: &LiteralString) -> Result<String, InterpolationError> {
    let parsed = parse_template(value.as_str())?;
    let mut result = String::new();

    for segment in parsed.segments() {
        match segment {
            TemplateSegment::Literal(value) => result.push_str(value),
            TemplateSegment::Resolver(call) => {
                return Err(InterpolationError::ResolverInLiteralString {
                    resolver: call.name.clone(),
                });
            }
        }
    }

    Ok(result)
}

pub fn validate_provider_install_arg(
    argument: &ProviderInstallArg,
) -> Result<(), InterpolationError> {
    let parsed = parse_template(argument.as_str())?;
    validate_template(&parsed, TemplateRole::ProviderInstallArg)
}

fn validate_template(
    template: &ParsedTemplate,
    role: TemplateRole,
) -> Result<(), InterpolationError> {
    for segment in template.segments() {
        let TemplateSegment::Resolver(call) = segment else {
            continue;
        };
        let resolver =
            lookup_resolver(&call.name).ok_or_else(|| InterpolationError::UnknownResolver {
                name: call.name.clone(),
            })?;

        if !resolver.availability.allows(role) {
            return Err(InterpolationError::ResolverUnavailable {
                resolver: call.name.clone(),
            });
        }
        if !(resolver.validate_payload)(&call.payload) {
            return Err(InterpolationError::InvalidResolverPayload {
                resolver: call.name.clone(),
                payload: call.payload.clone(),
            });
        }
        if (resolver.output)() == ListType::<StringType>::schema_type()
            && template.segments().len() != 1
        {
            return Err(InterpolationError::ListResolverMustOccupyArgument {
                resolver: call.name.clone(),
            });
        }
    }
    Ok(())
}

pub fn resolve_scalar_template(
    template: &ScalarTemplate,
    context: &ResolveContext<'_>,
) -> Result<String, InterpolationError> {
    let parsed = parse_template(template.as_str())?;
    validate_template(&parsed, TemplateRole::Scalar)?;
    resolve_scalar_segments(&parsed, context)
}

pub fn resolve_environment_patch(
    patch: &EnvironmentPatch,
    context: &ResolveContext<'_>,
) -> Result<EnvironmentPatch, InterpolationError> {
    Ok(EnvironmentPatch {
        path_prepend: patch
            .path_prepend
            .as_ref()
            .map(|values| resolve_scalar_values(values, context))
            .transpose()?,
        path_append: patch
            .path_append
            .as_ref()
            .map(|values| resolve_scalar_values(values, context))
            .transpose()?,
        variables: patch
            .variables
            .iter()
            .map(|(name, value)| {
                resolve_scalar_template(value, context)
                    .map(|value| (name.clone(), ScalarTemplate::from(value)))
            })
            .collect::<Result<_, _>>()?,
    })
}

pub fn resolve_exec_action(
    action: &ExecAction,
    context: &ResolveContext<'_>,
) -> Result<ExecAction, InterpolationError> {
    Ok(ExecAction {
        kind: action.kind,
        program: resolve_scalar_template(&action.program, context)?.into(),
        args: action
            .args
            .iter()
            .map(|argument| resolve_scalar_template(argument, context).map(ScalarTemplate::from))
            .collect::<Result<_, _>>()?,
        cwd: action
            .cwd
            .as_ref()
            .map(|cwd| resolve_scalar_template(cwd, context).map(ScalarTemplate::from))
            .transpose()?,
        env: action
            .env
            .as_ref()
            .map(|patch| resolve_environment_patch(patch, context))
            .transpose()?,
    })
}

pub fn resolve_provider_install_action(
    action: &ExecAction<ProviderInstallArg>,
    context: &ResolveContext<'_>,
) -> Result<ExecAction, InterpolationError> {
    let mut args = Vec::new();
    for argument in &action.args {
        args.extend(
            resolve_provider_install_arg(argument, context)?
                .into_iter()
                .map(ScalarTemplate::from),
        );
    }

    Ok(ExecAction {
        kind: action.kind,
        program: resolve_scalar_template(&action.program, context)?.into(),
        args,
        cwd: action
            .cwd
            .as_ref()
            .map(|cwd| resolve_scalar_template(cwd, context).map(ScalarTemplate::from))
            .transpose()?,
        env: action
            .env
            .as_ref()
            .map(|patch| resolve_environment_patch(patch, context))
            .transpose()?,
    })
}

fn resolve_scalar_values(
    values: &OneOrMany<ScalarTemplate>,
    context: &ResolveContext<'_>,
) -> Result<OneOrMany<ScalarTemplate>, InterpolationError> {
    match values {
        OneOrMany::One(value) => resolve_scalar_template(value, context)
            .map(ScalarTemplate::from)
            .map(OneOrMany::One),
        OneOrMany::Many(values) => values
            .iter()
            .map(|value| resolve_scalar_template(value, context).map(ScalarTemplate::from))
            .collect::<Result<_, _>>()
            .map(OneOrMany::Many),
    }
}

pub fn resolve_provider_install_arg(
    argument: &ProviderInstallArg,
    context: &ResolveContext<'_>,
) -> Result<Vec<String>, InterpolationError> {
    let parsed = parse_template(argument.as_str())?;
    validate_template(&parsed, TemplateRole::ProviderInstallArg)?;

    if let [TemplateSegment::Resolver(call)] = parsed.segments() {
        let resolver = lookup_resolver(&call.name).expect("validated resolver exists");
        if (resolver.output)() == ListType::<StringType>::schema_type() {
            let ResolverValue::List(values) = (resolver.resolve)(&call.payload, context)? else {
                unreachable!("resolver output matches its static definition");
            };
            return Ok(values);
        }
    }

    Ok(vec![resolve_scalar_segments(&parsed, context)?])
}

fn resolve_scalar_segments(
    template: &ParsedTemplate,
    context: &ResolveContext<'_>,
) -> Result<String, InterpolationError> {
    let mut result = String::new();
    for segment in template.segments() {
        match segment {
            TemplateSegment::Literal(value) => result.push_str(value),
            TemplateSegment::Resolver(call) => {
                let resolver = lookup_resolver(&call.name).expect("validated resolver exists");
                let ResolverValue::Scalar(value) = (resolver.resolve)(&call.payload, context)?
                else {
                    unreachable!("list resolvers cannot appear in a scalar template");
                };
                result.push_str(&value);
            }
        }
    }
    Ok(result)
}

fn parse_template(input: &str) -> Result<ParsedTemplate, InterpolationError> {
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut cursor = 0;

    while cursor < input.len() {
        let remaining = &input[cursor..];

        if remaining.starts_with(r"\${") {
            literal.push_str("${");
            cursor += 3;
            continue;
        }

        if remaining.starts_with("${") {
            if !literal.is_empty() {
                segments.push(TemplateSegment::Literal(std::mem::take(&mut literal)));
            }

            let call_offset = cursor;
            let body_offset = cursor + 2;
            let body_and_rest = &input[body_offset..];
            let Some(close_offset) = body_and_rest.find('}') else {
                return Err(InterpolationError::UnclosedResolver {
                    offset: call_offset,
                });
            };
            let body = &body_and_rest[..close_offset];

            if let Some(nested_offset) = body.find("${") {
                return Err(InterpolationError::NestedResolver {
                    offset: body_offset + nested_offset,
                });
            }

            let Some((name, payload)) = body.split_once(':') else {
                return Err(InterpolationError::MissingPayloadSeparator {
                    offset: call_offset,
                });
            };

            segments.push(TemplateSegment::Resolver(ResolverCall::new(name, payload)));
            cursor = body_offset + close_offset + 1;
            continue;
        }

        let character = remaining
            .chars()
            .next()
            .expect("cursor is before the end of the input");
        literal.push(character);
        cursor += character.len_utf8();
    }

    if !literal.is_empty() {
        segments.push(TemplateSegment::Literal(literal));
    }

    Ok(ParsedTemplate { segments })
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
        EnvironmentName, EnvironmentPatch, ExecAction, FlatListPart, ListType, LiteralString,
        LiteralStringSource, OneOrMany, ParsedStringForm, ProviderInstallArg,
        ProviderInstallArgSource, ScalarTemplate, SchemaType, StringExpression,
        StringExpressionSource, StringTemplatePart, StringType, TypedVariable,
    };

    use super::{
        DotPaths, InterpolationError, PackageContext, ResolveContext, ResolverCall,
        TemplateSegment, XdgPath, XdgPaths, parse_template, promote_literal_string,
        promote_provider_install_arg, promote_provider_install_args, promote_string_expression,
        resolve_environment_patch, resolve_exec_action, resolve_literal_string,
        resolve_provider_install_action, resolve_provider_install_arg, resolve_scalar_template,
        validate_provider_install_arg, validate_scalar_template,
    };

    fn environment(variables: &[(&str, &str)]) -> ExecutionEnvironment {
        let patch = EnvironmentPatch {
            path_prepend: None,
            path_append: None,
            variables: variables
                .iter()
                .map(|(name, value)| {
                    (
                        EnvironmentName::new(*name).expect("test name should be valid"),
                        ScalarTemplate::from(*value),
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
            InterpolationError::ResolverTypeMismatch {
                resolver: "package".into(),
                expected: SchemaType::String,
                actual: SchemaType::List(Box::new(SchemaType::String)),
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
    fn parser_treats_resolver_names_and_payloads_generically() {
        let parsed =
            parse_template(r"before-\${literal}-${env:HOME}/${dot:cwd}-${future:anything}-after")
                .expect("generic resolver syntax should parse");

        assert_eq!(
            parsed.segments(),
            &[
                TemplateSegment::Literal("before-${literal}-".into()),
                TemplateSegment::Resolver(ResolverCall::new("env", "HOME")),
                TemplateSegment::Literal("/".into()),
                TemplateSegment::Resolver(ResolverCall::new("dot", "cwd")),
                TemplateSegment::Literal("-".into()),
                TemplateSegment::Resolver(ResolverCall::new("future", "anything")),
                TemplateSegment::Literal("-after".into()),
            ]
        );
    }

    #[test]
    fn literal_strings_unescape_syntax_but_reject_resolvers() {
        assert_eq!(
            resolve_literal_string(&LiteralString::from(r"prefix-\${literal}"))
                .expect("escaped interpolation syntax should be literal"),
            "prefix-${literal}"
        );
        assert!(
            resolve_literal_string(&LiteralString::from("${env:HOME}"))
                .expect_err("literal strings must reject resolvers")
                .to_string()
                .contains("literal string")
        );
    }

    #[test]
    fn parser_rejects_an_unclosed_resolver_call() {
        let error = parse_template("prefix-${env:HOME")
            .expect_err("an unclosed resolver call should fail parsing");

        assert_eq!(error, InterpolationError::UnclosedResolver { offset: 7 });
    }

    #[test]
    fn parser_rejects_a_resolver_call_without_a_payload_separator() {
        let error = parse_template("${env}")
            .expect_err("a resolver call without a colon should fail parsing");

        assert_eq!(
            error,
            InterpolationError::MissingPayloadSeparator { offset: 0 }
        );
    }

    #[test]
    fn parser_rejects_nested_interpolation() {
        let error = parse_template("${env:${dot:cwd}}")
            .expect_err("nested interpolation should not be accepted");

        assert_eq!(error, InterpolationError::NestedResolver { offset: 6 });
    }

    #[test]
    fn dot_resolver_produces_invocation_paths() {
        let environment = environment(&[]);
        let xdg = xdg_paths(&[]);
        let context = ResolveContext::new(&environment, dot_paths(), &xdg);
        let template = ScalarTemplate::from("${dot:config}:${dot:config_dir}:${dot:cwd}");

        assert_eq!(
            resolve_scalar_template(&template, &context).expect("template should resolve"),
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
        assert_eq!(resolved.variables["TOOL_HOME"].as_str(), "/opt/tools");
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

        assert_eq!(resolved.program.as_str(), "probe-program");
        assert_eq!(
            resolved
                .args
                .iter()
                .map(ScalarTemplate::as_str)
                .collect::<Vec<_>>(),
            vec!["--config=/repo/dot.toml", "/home/tester/Documents"]
        );
        assert_eq!(resolved.cwd.as_ref().unwrap().as_str(), "/work");
        assert_eq!(
            resolved.env.as_ref().unwrap().variables["TOOL_HOME"].as_str(),
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
        let template = ScalarTemplate::from("${xdg:home}:${xdg:config}:${xdg:documents}");

        assert_eq!(
            resolve_scalar_template(&template, &context).expect("template should resolve"),
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
            let template = ScalarTemplate::from(format!("${{xdg:{payload}}}"));
            validate_scalar_template(&template)
                .unwrap_or_else(|error| panic!("xdg payload `{payload}` should be valid: {error}"));
        }
    }

    #[test]
    fn old_path_resolver_is_not_registered() {
        let template = ScalarTemplate::from("${path:cwd}");

        assert_eq!(
            validate_scalar_template(&template),
            Err(InterpolationError::UnknownResolver {
                name: "path".into()
            })
        );
    }

    #[test]
    fn validation_rejects_unknown_resolvers_without_evaluating_them() {
        let template = ScalarTemplate::from("${command:output}");

        assert_eq!(
            validate_scalar_template(&template),
            Err(InterpolationError::UnknownResolver {
                name: "command".into()
            })
        );
    }

    #[test]
    fn resolver_definitions_validate_their_own_payloads() {
        let template = ScalarTemplate::from("${xdg:repository}");

        assert_eq!(
            validate_scalar_template(&template),
            Err(InterpolationError::InvalidResolverPayload {
                resolver: "xdg".into(),
                payload: "repository".into(),
            })
        );
    }

    #[test]
    fn package_resolvers_are_available_only_to_provider_install_arguments() {
        let scalar = ScalarTemplate::from("${package:names}");
        let install_arg = ProviderInstallArg::from("${package:names}");

        assert_eq!(
            validate_scalar_template(&scalar),
            Err(InterpolationError::ResolverUnavailable {
                resolver: "package".into()
            })
        );
        validate_provider_install_arg(&install_arg)
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

        assert_eq!(
            resolve_provider_install_arg(&ProviderInstallArg::from("${package:names}"), &context,)
                .expect("package names should resolve"),
            vec![String::from("ripgrep"), String::from("zoxide")]
        );
        assert_eq!(
            resolve_provider_install_arg(
                &ProviderInstallArg::from("${package:provider_args}"),
                &context,
            )
            .expect("provider args should resolve"),
            vec![String::from("--locked")]
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
        let action = ExecAction::<ProviderInstallArg> {
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

        assert_eq!(resolved.program.as_str(), "brew");
        assert_eq!(
            resolved
                .args
                .iter()
                .map(ScalarTemplate::as_str)
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
        assert_eq!(resolved.cwd.as_ref().unwrap().as_str(), "/work");
    }

    #[test]
    fn list_resolvers_cannot_be_embedded_in_an_argument() {
        let argument = ProviderInstallArg::from("prefix-${package:names}");

        assert_eq!(
            validate_provider_install_arg(&argument),
            Err(InterpolationError::ListResolverMustOccupyArgument {
                resolver: "package".into()
            })
        );
    }
}
