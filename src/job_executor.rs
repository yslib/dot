use crate::action::ExecutionEnvironment;
use crate::action_runner::{CommandActionOutcome, CommandActionRunError, CommandActionRunner};
use crate::fetch_content::{FetchContentRunner, UreqHttpsTransport};
use crate::job_runner::ActionOutcome;
use crate::link::{self, LinkPhaseError, LinkReport};
use crate::plan::{
    PlannedAction, PlannedActionKind, PlannedLink, PlannedManualPackage, PlannedProvider,
    PlannedProviderInstall,
};
use crate::provider::{ProviderInstallStatus, ProviderRunner, ProviderStatus};

pub(crate) struct JobExecutor<'a> {
    provider_runner: ProviderRunner<'a>,
    command_action_runner: CommandActionRunner<'a>,
    fetch_transport: UreqHttpsTransport,
}

impl<'a> JobExecutor<'a> {
    pub(crate) fn new(environment: &'a ExecutionEnvironment) -> Self {
        Self {
            provider_runner: ProviderRunner::new(environment),
            command_action_runner: CommandActionRunner::new(environment),
            fetch_transport: UreqHttpsTransport::new(),
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
    ) -> Result<CommandActionOutcome, CommandActionRunError> {
        self.command_action_runner.run(package.install())
    }

    pub(crate) fn run_action(&self, action: &PlannedAction) -> ActionOutcome {
        match action.kind() {
            PlannedActionKind::Command(action) => {
                ActionOutcome::Command(self.command_action_runner.run(action))
            }
            PlannedActionKind::FetchContent(action) => ActionOutcome::FetchContent(
                FetchContentRunner::new(&self.fetch_transport).run(action),
            ),
        }
    }

    pub(crate) fn reconcile_links<'p>(
        &self,
        links: impl IntoIterator<Item = &'p PlannedLink>,
    ) -> Result<LinkReport, LinkPhaseError> {
        link::reconcile(links)
    }
}
