use std::collections::BTreeMap;
use std::sync::LazyLock;

use crate::schema::{
    EnvironmentName, ListType, ResolvedString, SchemaType, SchemaTypeMarker, StringType,
};

use super::{DotPath, InterpolationError, ResolveContext, TemplateRole, XdgPath};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResolverAvailability {
    Everywhere,
    ProviderInstallOnly,
}

impl ResolverAvailability {
    pub(super) fn allows(self, role: TemplateRole) -> bool {
        self == Self::Everywhere || role == TemplateRole::ProviderInstallArg
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ResolvedValue {
    String(ResolvedString),
    StringList(Vec<ResolvedString>),
}

trait Resolver: Send + Sync {
    fn namespace(&self) -> &'static str;
    fn output_type(&self) -> SchemaType;
    fn availability(&self) -> ResolverAvailability;
    fn validate_payload(&self, payload: &str) -> bool;
    fn resolve(
        &self,
        payload: &str,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedValue, InterpolationError>;
}

pub(super) struct ResolverEntry {
    output_type: SchemaType,
    resolver: Box<dyn Resolver>,
}

impl ResolverEntry {
    pub(super) fn output_type(&self) -> &SchemaType {
        &self.output_type
    }

    pub(super) fn availability(&self) -> ResolverAvailability {
        self.resolver.availability()
    }

    pub(super) fn validate_payload(&self, payload: &str) -> bool {
        self.resolver.validate_payload(payload)
    }

    pub(super) fn resolve(
        &self,
        payload: &str,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedValue, InterpolationError> {
        self.resolver.resolve(payload, context)
    }
}

type ResolverRegistry = BTreeMap<&'static str, ResolverEntry>;

static RESOLVERS: LazyLock<ResolverRegistry> = LazyLock::new(build_resolver_registry);

fn register<R>(registry: &mut ResolverRegistry, resolver: R)
where
    R: Resolver + 'static,
{
    let namespace = resolver.namespace();
    let output_type = resolver.output_type();
    let previous = registry.insert(
        namespace,
        ResolverEntry {
            output_type,
            resolver: Box::new(resolver),
        },
    );
    assert!(
        previous.is_none(),
        "duplicate resolver namespace `{namespace}`"
    );
}

fn build_resolver_registry() -> ResolverRegistry {
    let mut registry = BTreeMap::new();
    register(&mut registry, EnvResolver);
    register(&mut registry, DotResolver);
    register(&mut registry, XdgResolver);
    register(&mut registry, PackageResolver);
    registry
}

pub(super) fn lookup_resolver(namespace: &str) -> Option<&'static ResolverEntry> {
    RESOLVERS.get(namespace)
}

struct EnvResolver;

impl Resolver for EnvResolver {
    fn namespace(&self) -> &'static str {
        "env"
    }

    fn output_type(&self) -> SchemaType {
        StringType::schema_type()
    }

    fn availability(&self) -> ResolverAvailability {
        ResolverAvailability::Everywhere
    }

    fn validate_payload(&self, payload: &str) -> bool {
        EnvironmentName::new(payload).is_ok()
    }

    fn resolve(
        &self,
        payload: &str,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedValue, InterpolationError> {
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
        Ok(ResolvedValue::String(ResolvedString::from(value)))
    }
}

struct DotResolver;

impl Resolver for DotResolver {
    fn namespace(&self) -> &'static str {
        "dot"
    }

    fn output_type(&self) -> SchemaType {
        StringType::schema_type()
    }

    fn availability(&self) -> ResolverAvailability {
        ResolverAvailability::Everywhere
    }

    fn validate_payload(&self, payload: &str) -> bool {
        DotPath::from_payload(payload).is_some()
    }

    fn resolve(
        &self,
        payload: &str,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedValue, InterpolationError> {
        let path = context.dot.get(
            DotPath::from_payload(payload)
                .expect("payload was validated by the resolver definition"),
        );
        let value = path
            .to_str()
            .ok_or_else(|| InterpolationError::NonUnicodePath {
                name: payload.to_owned(),
            })?;
        Ok(ResolvedValue::String(ResolvedString::from(value)))
    }
}

struct XdgResolver;

impl Resolver for XdgResolver {
    fn namespace(&self) -> &'static str {
        "xdg"
    }

    fn output_type(&self) -> SchemaType {
        StringType::schema_type()
    }

    fn availability(&self) -> ResolverAvailability {
        ResolverAvailability::Everywhere
    }

    fn validate_payload(&self, payload: &str) -> bool {
        XdgPath::from_payload(payload).is_some()
    }

    fn resolve(
        &self,
        payload: &str,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedValue, InterpolationError> {
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
        Ok(ResolvedValue::String(ResolvedString::from(value)))
    }
}

struct PackageResolver;

impl Resolver for PackageResolver {
    fn namespace(&self) -> &'static str {
        "package"
    }

    fn output_type(&self) -> SchemaType {
        ListType::<StringType>::schema_type()
    }

    fn availability(&self) -> ResolverAvailability {
        ResolverAvailability::ProviderInstallOnly
    }

    fn validate_payload(&self, payload: &str) -> bool {
        matches!(payload, "names" | "provider_args")
    }

    fn resolve(
        &self,
        payload: &str,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedValue, InterpolationError> {
        let package = context
            .package
            .ok_or(InterpolationError::MissingPackageContext)?;
        let values = match payload {
            "names" => package.names,
            "provider_args" => package.provider_args,
            _ => unreachable!("payload was validated by the resolver"),
        };
        Ok(ResolvedValue::StringList(
            values.iter().cloned().map(ResolvedString::from).collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::schema::{ListType, SchemaTypeMarker, StringType};

    use super::{
        EnvResolver, ResolverAvailability, build_resolver_registry, lookup_resolver, register,
    };

    #[test]
    fn registry_contains_each_builtin_namespace_in_sorted_order() {
        let registry = build_resolver_registry();

        assert_eq!(
            registry.keys().copied().collect::<Vec<_>>(),
            ["dot", "env", "package", "xdg"]
        );
    }

    #[test]
    fn builtin_output_types_match_their_declared_values() {
        let registry = build_resolver_registry();

        for namespace in ["env", "dot", "xdg"] {
            assert_eq!(
                registry[namespace].output_type(),
                &StringType::schema_type()
            );
        }
        assert_eq!(
            registry["package"].output_type(),
            &ListType::<StringType>::schema_type()
        );
    }

    #[test]
    fn builtin_availability_matches_its_template_role() {
        let registry = build_resolver_registry();

        assert_eq!(
            registry["env"].availability(),
            ResolverAvailability::Everywhere
        );
        assert_eq!(
            registry["package"].availability(),
            ResolverAvailability::ProviderInstallOnly
        );
    }

    #[test]
    #[should_panic(expected = "duplicate resolver namespace `env`")]
    fn duplicate_namespaces_are_rejected() {
        let mut registry = Default::default();

        register(&mut registry, EnvResolver);
        register(&mut registry, EnvResolver);
    }

    #[test]
    fn builtin_resolvers_validate_only_their_declared_payloads() {
        let registry = build_resolver_registry();

        assert!(registry["env"].validate_payload("HOME"));
        assert!(!registry["env"].validate_payload(""));
        assert!(registry["dot"].validate_payload("cwd"));
        assert!(!registry["dot"].validate_payload("home"));
        assert!(registry["xdg"].validate_payload("executable"));
        assert!(!registry["xdg"].validate_payload("repository"));
        assert!(registry["package"].validate_payload("names"));
        assert!(registry["package"].validate_payload("provider_args"));
        assert!(!registry["package"].validate_payload("name"));
    }

    #[test]
    fn lookup_finds_each_builtin_and_rejects_unknown_namespaces() {
        for namespace in ["env", "dot", "xdg", "package"] {
            assert!(lookup_resolver(namespace).is_some());
        }
        assert!(lookup_resolver("unknown").is_none());
    }
}
