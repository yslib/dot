use std::collections::{BTreeMap, btree_map::Entry};
use std::fmt::Debug;

use super::command_action::CommandActionRunner;
use super::command_action::{
    CommandActionOutcome, CommandActionRunError, FetchContentError, FetchContentOutcome,
};
use super::fetch_content::{FetchContentRunner, UreqHttpsTransport};
use super::link::{self, LinkReport};
use super::link::{LinkError, LinkOutcome, LinkPhaseError};
use super::plan::{
    ExecutionPlan, PlannedAction, PlannedActionKind, PlannedJob, PlannedLink, PlannedManualPackage,
    PlannedPackage, PlannedProvider, PlannedProviderInstall,
};
use super::provider::{
    ProviderError, ProviderInstallError, ProviderInstallOutcome, ProviderInstallStatus,
    ProviderOutcome, ProviderRunner, ProviderStatus,
};
use crate::interpolation::ExecutionEnvironment;
use crate::job::JobId;
use crate::schema::Identifier;

#[derive(Debug)]
pub enum BlockReason {
    ProviderUnavailable { provider: Identifier },
    LinkPhase { message: String },
}

#[derive(Debug)]
pub enum JobOutcome {
    Provider(Result<ProviderOutcome, ProviderError>),
    ProviderPackage(Result<ProviderInstallOutcome, ProviderInstallError>),
    ManualPackage(Result<CommandActionOutcome, CommandActionRunError>),
    Action(ActionOutcome),
    Link(Result<LinkOutcome, LinkError>),
}

#[derive(Debug)]
pub enum ActionOutcome {
    Command(Result<CommandActionOutcome, CommandActionRunError>),
    FetchContent(Result<FetchContentOutcome, FetchContentError>),
}

impl ActionOutcome {
    const fn is_succeeded(&self) -> bool {
        match self {
            Self::Command(result) => result.is_ok(),
            Self::FetchContent(result) => result.is_ok(),
        }
    }
}

#[derive(Debug)]
pub enum JobState {
    Completed(JobOutcome),
    Blocked(BlockReason),
}

impl JobState {
    fn is_succeeded(&self) -> bool {
        match self {
            Self::Completed(outcome) => match outcome {
                JobOutcome::Provider(outcome) => outcome.is_ok(),
                JobOutcome::ProviderPackage(outcome) => {
                    matches!(outcome, Ok(ProviderInstallOutcome::Executed { .. }))
                }
                JobOutcome::ManualPackage(outcome) => outcome.is_ok(),
                JobOutcome::Action(outcome) => outcome.is_succeeded(),
                JobOutcome::Link(outcome) => outcome.is_ok(),
            },
            Self::Blocked(_) => false,
        }
    }
}

fn insert_unique_result<K, V>(results: &mut BTreeMap<K, V>, id: K, state: V)
where
    K: Debug + Ord,
{
    match results.entry(id) {
        Entry::Vacant(entry) => {
            entry.insert(state);
        }
        Entry::Occupied(entry) => {
            panic!("duplicate job result for `{:?}`", entry.key());
        }
    }
}

fn assert_result_count(expected: usize, actual: usize) {
    assert_eq!(
        expected, actual,
        "job result count mismatch: expected {expected} selected jobs, got {actual} results"
    );
}

fn assert_link_projection(expected: &[&str], actual: &[&str]) {
    assert_eq!(
        expected.len(),
        actual.len(),
        "link result count mismatch: expected {} selected links, got {} results",
        expected.len(),
        actual.len()
    );
    for (index, (expected_id, actual_id)) in expected.iter().zip(actual).enumerate() {
        assert_eq!(
            actual_id, expected_id,
            "link result identity mismatch at index {index}: expected `{expected_id}`, got `{actual_id}`"
        );
    }
}

#[derive(Debug)]
pub struct JobExecutionReport {
    results: BTreeMap<JobId, JobState>,
    link_phase_error: Option<LinkPhaseError>,
}

impl JobExecutionReport {
    pub fn get(&self, id: &JobId) -> Option<&JobState> {
        self.results.get(id)
    }

    pub fn len(&self) -> usize {
        self.results.len()
    }

    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }

    pub fn all_succeeded(&self) -> bool {
        self.results.values().all(JobState::is_succeeded)
    }

    pub const fn link_phase_error(&self) -> Option<&LinkPhaseError> {
        self.link_phase_error.as_ref()
    }
}

struct JobExecutor<'a> {
    provider_runner: ProviderRunner<'a>,
    command_action_runner: CommandActionRunner<'a>,
    fetch_transport: UreqHttpsTransport,
}

impl<'a> JobExecutor<'a> {
    fn new(environment: &'a ExecutionEnvironment) -> Self {
        Self {
            provider_runner: ProviderRunner::new(environment),
            command_action_runner: CommandActionRunner::new(environment),
            fetch_transport: UreqHttpsTransport::new(),
        }
    }

    fn ensure_provider(&self, provider: &PlannedProvider) -> ProviderStatus {
        self.provider_runner.ensure(provider)
    }

    fn install_provider_package(
        &self,
        install: &PlannedProviderInstall,
        provider: &ProviderStatus,
    ) -> ProviderInstallStatus {
        self.provider_runner.install(install, provider)
    }

    fn install_manual_package(
        &self,
        package: &PlannedManualPackage,
    ) -> Result<CommandActionOutcome, CommandActionRunError> {
        self.command_action_runner.run(package.install())
    }

    fn run_action(&self, action: &PlannedAction) -> ActionOutcome {
        match action.kind() {
            PlannedActionKind::Command(action) => {
                ActionOutcome::Command(self.command_action_runner.run(action))
            }
            PlannedActionKind::FetchContent(action) => ActionOutcome::FetchContent(
                FetchContentRunner::new(&self.fetch_transport).run(action),
            ),
        }
    }

    fn reconcile_links<'p>(
        &self,
        links: impl IntoIterator<Item = &'p PlannedLink>,
    ) -> Result<LinkReport, LinkPhaseError> {
        link::reconcile(links)
    }
}

pub struct JobRunner<'a> {
    executor: JobExecutor<'a>,
}

impl<'a> JobRunner<'a> {
    pub fn new(environment: &'a ExecutionEnvironment) -> Self {
        Self {
            executor: JobExecutor::new(environment),
        }
    }

    pub fn run(&self, plan: &ExecutionPlan) -> JobExecutionReport {
        let expected_result_count = plan.jobs().len();
        let mut results = BTreeMap::new();
        let mut provider_outputs = BTreeMap::<JobId, ProviderStatus>::new();
        let mut links = Vec::new();

        for job in plan.jobs() {
            let job_id = job.id();
            let state = match job {
                PlannedJob::Provider(provider) => {
                    insert_unique_result(
                        &mut provider_outputs,
                        job_id.clone(),
                        self.executor.ensure_provider(provider),
                    );
                    None
                }
                PlannedJob::Package(PlannedPackage::Provider(package)) => {
                    let provider_id = package.provider_id().clone();
                    let provider_job_id = JobId::Provider(provider_id.clone());
                    let state = match provider_outputs.get(&provider_job_id) {
                        Some(status) if status.is_ready() => {
                            JobState::Completed(JobOutcome::ProviderPackage(
                                self.executor
                                    .install_provider_package(package, status)
                                    .into_outcome(),
                            ))
                        }
                        _ => JobState::Blocked(BlockReason::ProviderUnavailable {
                            provider: provider_id,
                        }),
                    };
                    Some(state)
                }
                PlannedJob::Package(PlannedPackage::Manual(package)) => Some(JobState::Completed(
                    JobOutcome::ManualPackage(self.executor.install_manual_package(package)),
                )),
                PlannedJob::Action(action) => Some(JobState::Completed(JobOutcome::Action(
                    self.executor.run_action(action),
                ))),
                PlannedJob::Link(link) => {
                    links.push((job_id.clone(), link));
                    None
                }
            };

            if let Some(state) = state {
                insert_unique_result(&mut results, job_id, state);
            }
        }

        let mut link_phase_error = None;
        if !links.is_empty() {
            match self
                .executor
                .reconcile_links(links.iter().map(|(_, link)| *link))
            {
                Ok(report) => {
                    {
                        let expected_ids =
                            links.iter().map(|(_, link)| link.id()).collect::<Vec<_>>();
                        let actual_ids = report
                            .results()
                            .iter()
                            .map(|result| result.id())
                            .collect::<Vec<_>>();
                        assert_link_projection(&expected_ids, &actual_ids);
                    }
                    let link_results = report.into_results();
                    for ((job_id, _), result) in links.into_iter().zip(link_results) {
                        let (_, outcome) = result.into_parts();
                        insert_unique_result(
                            &mut results,
                            job_id,
                            JobState::Completed(JobOutcome::Link(outcome)),
                        );
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    for (job_id, _) in links {
                        insert_unique_result(
                            &mut results,
                            job_id,
                            JobState::Blocked(BlockReason::LinkPhase {
                                message: message.clone(),
                            }),
                        );
                    }
                    link_phase_error = Some(error);
                }
            }
        }

        for (job_id, status) in provider_outputs {
            insert_unique_result(
                &mut results,
                job_id,
                JobState::Completed(JobOutcome::Provider(status.into_outcome())),
            );
        }

        assert_result_count(expected_result_count, results.len());
        JobExecutionReport {
            results,
            link_phase_error,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{assert_link_projection, assert_result_count, insert_unique_result};

    #[test]
    #[should_panic(expected = "link result count mismatch")]
    fn link_projection_rejects_a_result_count_mismatch() {
        assert_link_projection(&["first"], &["first", "second"]);
    }

    #[test]
    #[should_panic(expected = "link result identity mismatch at index 0")]
    fn link_projection_rejects_results_in_the_wrong_order() {
        assert_link_projection(&["first", "second"], &["second", "first"]);
    }

    #[test]
    #[should_panic(expected = "duplicate job result")]
    fn unique_result_insertion_rejects_a_duplicate_key() {
        let mut results = BTreeMap::new();

        insert_unique_result(&mut results, "duplicate", 1);
        insert_unique_result(&mut results, "duplicate", 2);
    }

    #[test]
    #[should_panic(expected = "job result count mismatch")]
    fn result_count_rejects_an_incomplete_report() {
        assert_result_count(2, 1);
    }
}
