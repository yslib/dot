use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

use serde::{Deserialize, Deserializer, de};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecordTypeId(&'static str);

impl RecordTypeId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StringRefinementTypeId(&'static str);

impl StringRefinementTypeId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StringKeyType {
    String,
    Refinement(StringRefinementTypeId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaType {
    String,
    Integer,
    Float,
    Boolean,
    OffsetDateTime,
    LocalDateTime,
    LocalDate,
    LocalTime,
    List(Box<SchemaType>),
    Map(StringKeyType, Box<SchemaType>),
    Record(RecordTypeId),
}

pub trait SchemaTypeMarker {
    type Resolved;

    fn schema_type() -> SchemaType;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StringType;

impl SchemaTypeMarker for StringType {
    type Resolved = ResolvedString;

    fn schema_type() -> SchemaType {
        SchemaType::String
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListType<T>(PhantomData<fn() -> T>);

impl<T> SchemaTypeMarker for ListType<T>
where
    T: SchemaTypeMarker,
{
    type Resolved = Vec<T::Resolved>;

    fn schema_type() -> SchemaType {
        SchemaType::List(Box::new(T::schema_type()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResolvedString(String);

impl ResolvedString {
    pub fn value(&self) -> &str {
        &self.0
    }
}

impl From<String> for ResolvedString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ResolvedString {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

pub type Entries<T> = BTreeMap<Identifier, T>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identifier(String);

impl Identifier {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdentifierError::Empty);
        }
        if value.contains("${") {
            return Err(IdentifierError::Interpolation);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Identifier {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for Identifier {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<String> for Identifier {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for Identifier {
    type Error = IdentifierError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for Identifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentifierError {
    Empty,
    Interpolation,
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("an identifier cannot be empty"),
            Self::Interpolation => {
                formatter.write_str("an identifier cannot contain interpolation")
            }
        }
    }
}

impl Error for IdentifierError {}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnvironmentName(String);

impl EnvironmentName {
    pub fn new(value: impl Into<String>) -> Result<Self, EnvironmentNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(EnvironmentNameError::Empty);
        }
        if value.contains('=') {
            return Err(EnvironmentNameError::EqualsSign);
        }
        if value.contains("${") {
            return Err(EnvironmentNameError::Interpolation);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for EnvironmentName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for EnvironmentName {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for EnvironmentName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<String> for EnvironmentName {
    type Error = EnvironmentNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for EnvironmentName {
    type Error = EnvironmentNameError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for EnvironmentName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentNameError {
    Empty,
    EqualsSign,
    Interpolation,
}

impl fmt::Display for EnvironmentNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("an environment name cannot be empty"),
            Self::EqualsSign => formatter.write_str("an environment name cannot contain `=`"),
            Self::Interpolation => {
                formatter.write_str("an environment name cannot contain interpolation")
            }
        }
    }
}

impl Error for EnvironmentNameError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedStringForm {
    Literal(StringLiteralSource),
    Template(ParsedTemplate),
    Variable(UntypedVariableReference),
    Malformed(ExpressionParseError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringLiteralSource(String);

impl StringLiteralSource {
    pub fn value(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedTemplate {
    parts: Vec<ParsedTemplatePart>,
}

impl ParsedTemplate {
    pub fn parts(&self) -> &[ParsedTemplatePart] {
        &self.parts
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedTemplatePart {
    Literal(String),
    Variable(UntypedVariableReference),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UntypedVariableReference {
    resolver: String,
    payload: String,
}

impl UntypedVariableReference {
    pub fn resolver(&self) -> &str {
        &self.resolver
    }

    pub fn payload(&self) -> &str {
        &self.payload
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpressionParseError {
    UnclosedResolver { offset: usize },
    MissingPayloadSeparator { offset: usize },
    NestedResolver { offset: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedVariable<T> {
    reference: UntypedVariableReference,
    marker: PhantomData<fn() -> T>,
}

impl<T> TypedVariable<T> {
    pub fn reference(&self) -> &UntypedVariableReference {
        &self.reference
    }

    pub(crate) fn validated(reference: UntypedVariableReference) -> Self {
        Self {
            reference,
            marker: PhantomData,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiteralString(String);

impl LiteralString {
    pub fn value(&self) -> &str {
        &self.0
    }

    pub(crate) fn validated(value: String) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StringTemplatePart<V> {
    Literal(String),
    Variable(V),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringTemplate<V> {
    parts: Vec<StringTemplatePart<V>>,
}

impl<V> StringTemplate<V> {
    pub fn parts(&self) -> &[StringTemplatePart<V>] {
        &self.parts
    }

    pub(crate) fn validated(parts: Vec<StringTemplatePart<V>>) -> Self {
        Self { parts }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StringExpression {
    Literal(LiteralString),
    Template(StringTemplate<TypedVariable<StringType>>),
    Variable(TypedVariable<StringType>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlatListPart<T, E> {
    One(E),
    Many(TypedVariable<ListType<T>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlatListExpression<T, E> {
    parts: Vec<FlatListPart<T, E>>,
}

impl<T, E> FlatListExpression<T, E> {
    pub fn parts(&self) -> &[FlatListPart<T, E>] {
        &self.parts
    }

    pub(crate) fn validated(parts: Vec<FlatListPart<T, E>>) -> Self {
        Self { parts }
    }
}

pub type ProviderInstallArgs = FlatListExpression<StringType, StringExpression>;

fn classify_string(source: &str) -> ParsedStringForm {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut cursor = 0;

    while cursor < source.len() {
        let remaining = &source[cursor..];

        if remaining.starts_with(r"\${") {
            literal.push_str("${");
            cursor += 3;
            continue;
        }

        if remaining.starts_with("${") {
            if !literal.is_empty() {
                parts.push(ParsedTemplatePart::Literal(std::mem::take(&mut literal)));
            }

            let call_offset = cursor;
            let body_offset = cursor + 2;
            let body_and_rest = &source[body_offset..];
            let Some(close_offset) = body_and_rest.find('}') else {
                return ParsedStringForm::Malformed(ExpressionParseError::UnclosedResolver {
                    offset: call_offset,
                });
            };
            let body = &body_and_rest[..close_offset];

            if let Some(nested_offset) = body.find("${") {
                return ParsedStringForm::Malformed(ExpressionParseError::NestedResolver {
                    offset: body_offset + nested_offset,
                });
            }

            let Some((resolver, payload)) = body.split_once(':') else {
                return ParsedStringForm::Malformed(
                    ExpressionParseError::MissingPayloadSeparator {
                        offset: call_offset,
                    },
                );
            };

            parts.push(ParsedTemplatePart::Variable(UntypedVariableReference {
                resolver: resolver.to_owned(),
                payload: payload.to_owned(),
            }));
            cursor = body_offset + close_offset + 1;
            continue;
        }

        let character = remaining
            .chars()
            .next()
            .expect("cursor is before the end of the source");
        literal.push(character);
        cursor += character.len_utf8();
    }

    if !literal.is_empty() {
        parts.push(ParsedTemplatePart::Literal(literal));
    }

    match parts.as_slice() {
        [] => ParsedStringForm::Literal(StringLiteralSource(String::new())),
        [ParsedTemplatePart::Literal(value)] => {
            ParsedStringForm::Literal(StringLiteralSource(value.clone()))
        }
        [ParsedTemplatePart::Variable(reference)] => ParsedStringForm::Variable(reference.clone()),
        _ => ParsedStringForm::Template(ParsedTemplate { parts }),
    }
}

macro_rules! source_string_type {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct $name {
            source: String,
            parsed: ParsedStringForm,
        }

        impl $name {
            pub fn parsed(&self) -> &ParsedStringForm {
                &self.parsed
            }

            pub fn source_spelling(&self) -> &str {
                &self.source
            }
        }

        impl From<String> for $name {
            fn from(source: String) -> Self {
                let parsed = classify_string(&source);
                Self { source, parsed }
            }
        }

        impl From<&str> for $name {
            fn from(source: &str) -> Self {
                Self::from(source.to_owned())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Ok(Self::from(String::deserialize(deserializer)?))
            }
        }
    };
}

source_string_type!(LiteralStringSource);
source_string_type!(StringExpressionSource);
source_string_type!(ProviderInstallArgSource);

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub targets: Entries<Target>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub platform: PlatformConstraint,
    #[serde(default)]
    pub providers: Entries<Provider>,
    #[serde(default)]
    pub packages: Entries<Package>,
    #[serde(default)]
    pub links: Entries<Link>,
    #[serde(default)]
    pub actions: Entries<Action>,
    #[serde(default)]
    pub profiles: Entries<Profile>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default)]
    pub providers: Entries<Provider>,
    #[serde(default)]
    pub packages: Entries<Package>,
    #[serde(default)]
    pub links: Entries<Link>,
    #[serde(default)]
    pub actions: Entries<Action>,
    #[serde(default)]
    pub profiles: Entries<Profile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformConstraint {
    pub os: OneOrMany<Identifier>,
    pub arch: Option<OneOrMany<Identifier>>,
    pub distro: Option<OneOrMany<Identifier>>,
    pub distro_family: Option<OneOrMany<Identifier>>,
    pub environment: Option<OneOrMany<Identifier>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Package {
    Provider(ProviderPackage),
    Manual(Box<ManualPackage>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum ProviderPackage {
    Batch(BatchProviderPackage),
    Single(SingleProviderPackage),
}

impl ProviderPackage {
    pub fn provider(&self) -> &Identifier {
        match self {
            Self::Single(package) => &package.provider,
            Self::Batch(package) => &package.provider,
        }
    }

    pub fn provider_args(&self) -> Option<&[LiteralStringSource]> {
        match self {
            Self::Single(package) => package.provider_args.as_deref(),
            Self::Batch(package) => package.provider_args.as_deref(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SingleProviderPackage {
    pub provider: Identifier,
    pub provider_args: Option<Vec<LiteralStringSource>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchProviderPackage {
    pub provider: Identifier,
    pub names: Vec<Identifier>,
    pub provider_args: Option<Vec<LiteralStringSource>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualPackage {
    pub install: Action,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub probe: ExecAction,
    pub activate: Option<EnvironmentPatch>,
    pub ensure: Option<OneOrMany<ExecAction>>,
    pub install: ExecAction<StringExpressionSource, ProviderInstallArgSource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "S: Deserialize<'de>"))]
pub struct EnvironmentPatch<S = StringExpressionSource> {
    pub path_prepend: Option<OneOrMany<S>>,
    pub path_append: Option<OneOrMany<S>>,
    #[serde(default)]
    pub variables: BTreeMap<EnvironmentName, S>,
}

impl<S> Default for EnvironmentPatch<S> {
    fn default() -> Self {
        Self {
            path_prepend: None,
            path_append: None,
            variables: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "S: Deserialize<'de>, A: Deserialize<'de>")
)]
pub struct ExecAction<S = StringExpressionSource, A = S> {
    #[serde(rename = "type")]
    pub kind: Option<ExecActionType>,
    pub program: S,
    #[serde(default)]
    pub args: Vec<A>,
    pub cwd: Option<S>,
    pub env: Option<EnvironmentPatch<S>>,
}

pub type SourceExecAction = ExecAction<StringExpressionSource, StringExpressionSource>;
pub type ResolvedEnvironmentPatch = EnvironmentPatch<ResolvedString>;
pub type ResolvedExecAction = ExecAction<ResolvedString, ResolvedString>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecActionType {
    Exec,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub source: StringExpressionSource,
    pub target: StringExpressionSource,
    pub on_conflict: Option<LinkConflict>,
    pub on_missing_parent: Option<LinkMissingParent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkConflict {
    Error,
    ReplaceLink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkMissingParent {
    Create,
    Skip,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "S: Deserialize<'de>, A: Deserialize<'de>")
)]
pub struct Action<S = StringExpressionSource, A = S> {
    pub check: Option<ExecAction<S, A>>,
    pub exec: ExecAction<S, A>,
}

pub type SourceAction = Action<StringExpressionSource, StringExpressionSource>;
pub type ResolvedAction = Action<ResolvedString, ResolvedString>;
