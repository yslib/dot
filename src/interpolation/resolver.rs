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
