use std::collections::BTreeMap;
use std::sync::LazyLock;

use crate::schema::{EnvironmentName, ListType, SchemaType, SchemaTypeMarker, StringType};

use super::TemplateRole;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResolverKind {
    Environment,
    DotPath,
    XdgPath,
    Package,
}

pub(super) struct ResolverEntry {
    kind: ResolverKind,
    output_type: SchemaType,
    availability: ResolverAvailability,
}

impl ResolverEntry {
    pub(super) fn output_type(&self) -> &SchemaType {
        &self.output_type
    }

    pub(super) const fn availability(&self) -> ResolverAvailability {
        self.availability
    }

    pub(super) fn validate_payload(&self, payload: &str) -> bool {
        match self.kind {
            ResolverKind::Environment => EnvironmentName::new(payload).is_ok(),
            ResolverKind::DotPath => matches!(payload, "config_dir" | "real_config_dir" | "cwd"),
            ResolverKind::XdgPath => matches!(
                payload,
                "home"
                    | "config"
                    | "config_local"
                    | "data"
                    | "data_local"
                    | "cache"
                    | "state"
                    | "runtime"
                    | "executable"
                    | "documents"
            ),
            ResolverKind::Package => matches!(payload, "names" | "provider_args"),
        }
    }

    pub(crate) const fn kind(&self) -> ResolverKind {
        self.kind
    }
}

type ResolverRegistry = BTreeMap<&'static str, ResolverEntry>;

static RESOLVERS: LazyLock<ResolverRegistry> = LazyLock::new(build_resolver_registry);

fn build_resolver_registry() -> ResolverRegistry {
    BTreeMap::from([
        (
            "dot",
            ResolverEntry {
                kind: ResolverKind::DotPath,
                output_type: StringType::schema_type(),
                availability: ResolverAvailability::Everywhere,
            },
        ),
        (
            "env",
            ResolverEntry {
                kind: ResolverKind::Environment,
                output_type: StringType::schema_type(),
                availability: ResolverAvailability::Everywhere,
            },
        ),
        (
            "package",
            ResolverEntry {
                kind: ResolverKind::Package,
                output_type: ListType::<StringType>::schema_type(),
                availability: ResolverAvailability::ProviderInstallOnly,
            },
        ),
        (
            "xdg",
            ResolverEntry {
                kind: ResolverKind::XdgPath,
                output_type: StringType::schema_type(),
                availability: ResolverAvailability::Everywhere,
            },
        ),
    ])
}

pub(super) fn lookup_resolver(namespace: &str) -> Option<&'static ResolverEntry> {
    RESOLVERS.get(namespace)
}

#[cfg(test)]
mod tests {
    use crate::schema::{ListType, SchemaTypeMarker, StringType};

    use super::{ResolverAvailability, build_resolver_registry, lookup_resolver};

    #[test]
    fn registry_declares_builtin_types_and_availability() {
        let registry = build_resolver_registry();

        assert_eq!(
            registry.keys().copied().collect::<Vec<_>>(),
            ["dot", "env", "package", "xdg"]
        );
        for namespace in ["env", "dot", "xdg"] {
            assert_eq!(
                registry[namespace].output_type(),
                &StringType::schema_type()
            );
            assert_eq!(
                registry[namespace].availability(),
                ResolverAvailability::Everywhere
            );
        }
        assert_eq!(
            registry["package"].output_type(),
            &ListType::<StringType>::schema_type()
        );
        assert_eq!(
            registry["package"].availability(),
            ResolverAvailability::ProviderInstallOnly
        );
    }

    #[test]
    fn builtin_payload_validation_is_schema_only() {
        let registry = build_resolver_registry();

        assert!(registry["env"].validate_payload("HOME"));
        assert!(!registry["env"].validate_payload(""));
        for payload in ["config_dir", "real_config_dir", "cwd"] {
            assert!(registry["dot"].validate_payload(payload));
        }
        assert!(!registry["dot"].validate_payload("config"));
        assert!(!registry["dot"].validate_payload("real_config"));
        assert!(!registry["dot"].validate_payload("home"));
        assert!(registry["xdg"].validate_payload("executable"));
        assert!(!registry["xdg"].validate_payload("repository"));
        assert!(registry["package"].validate_payload("names"));
        assert!(registry["package"].validate_payload("provider_args"));
        assert!(!registry["package"].validate_payload("name"));
    }

    #[test]
    fn lookup_rejects_unknown_namespaces() {
        for namespace in ["env", "dot", "xdg", "package"] {
            assert!(lookup_resolver(namespace).is_some());
        }
        assert!(lookup_resolver("unknown").is_none());
    }
}
