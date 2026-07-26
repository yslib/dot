use crate::action::ExecutionEnvironment;
use crate::action_runner::{ActionOutcome, ActionRunError, ActionRunner};
use crate::link::{self, LinkPhaseError, LinkReport};
use crate::plan::{
    PlannedAction, PlannedLink, PlannedManualPackage, PlannedProvider, PlannedProviderInstall,
};
use crate::provider::{ProviderInstallStatus, ProviderRunner, ProviderStatus};

#[derive(Clone, Copy, Debug)]
pub(crate) struct JobExecutor<'a> {
    provider_runner: ProviderRunner<'a>,
    action_runner: ActionRunner<'a>,
}

impl<'a> JobExecutor<'a> {
    pub(crate) const fn new(environment: &'a ExecutionEnvironment) -> Self {
        Self {
            provider_runner: ProviderRunner::new(environment),
            action_runner: ActionRunner::new(environment),
        }
    }

    pub(crate) fn ensure_provider(&self, provider: &PlannedProvider) -> ProviderStatus {
        self.provider_runner.ensure(provider)
    }

    pub(crate) fn install_provider_package(
        &self,
        install: &PlannedProviderInstall,
        provider: &ProviderStatus,
    ) -> ProviderInstallStatus {
        self.provider_runner.install(install, provider)
    }

    pub(crate) fn install_manual_package(
        &self,
        package: &PlannedManualPackage,
    ) -> Result<ActionOutcome, ActionRunError> {
        self.action_runner.run(package.install())
    }

    pub(crate) fn run_action(
        &self,
        action: &PlannedAction,
    ) -> Result<ActionOutcome, ActionRunError> {
        self.action_runner.run(action.action())
    }

    pub(crate) fn reconcile_links<'p>(
        &self,
        links: impl IntoIterator<Item = &'p PlannedLink>,
    ) -> Result<LinkReport, LinkPhaseError> {
        link::reconcile(links)
    }
}
