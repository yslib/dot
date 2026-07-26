use std::collections::BTreeMap;

use crate::action::ExecutionEnvironment;
use crate::action_runner::{ActionOutcome, ActionRunError};
use crate::job::JobId;
use crate::job_executor::JobExecutor;
use crate::link::{LinkError, LinkOutcome, LinkPhaseError};
use crate::plan::{PlannedJob, PlannedPackage, SelectedExecutionPlan};
use crate::provider::{ProviderInstallStatus, ProviderStatus};
use crate::schema::Identifier;

#[derive(Debug)]
pub enum BlockReason {
    ProviderUnavailable { provider: Identifier },
    LinkPhase { message: String },
}

#[derive(Debug)]
pub enum JobOutcome {
    Provider(ProviderStatus),
    ProviderPackage(ProviderInstallStatus),
    ManualPackage(Result<ActionOutcome, ActionRunError>),
    Action(Result<ActionOutcome, ActionRunError>),
    Link(Result<LinkOutcome, LinkError>),
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
                JobOutcome::Provider(status) => status.is_ready(),
                JobOutcome::ProviderPackage(status) => status.is_succeeded(),
                JobOutcome::ManualPackage(outcome) | JobOutcome::Action(outcome) => outcome.is_ok(),
                JobOutcome::Link(outcome) => outcome.is_ok(),
            },
            Self::Blocked(_) => false,
        }
    }
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

#[derive(Clone, Copy, Debug)]
pub struct JobRunner<'a> {
    executor: JobExecutor<'a>,
}

impl<'a> JobRunner<'a> {
    pub const fn new(environment: &'a ExecutionEnvironment) -> Self {
        Self {
            executor: JobExecutor::new(environment),
        }
    }

    pub fn run(&self, selected: &SelectedExecutionPlan<'_>) -> JobExecutionReport {
        let jobs = selected.jobs().collect::<Vec<_>>();
        let mut results = BTreeMap::new();
        let mut links = Vec::new();

        for job in jobs {
            let job_id = job.id();
            let state = match job {
                PlannedJob::Provider(provider) => Some(JobState::Completed(JobOutcome::Provider(
                    self.executor.ensure_provider(provider),
                ))),
                PlannedJob::Package(PlannedPackage::Provider(package)) => {
                    let provider_id = package.provider_id().clone();
                    let provider_job_id = JobId::Provider(provider_id.clone());
                    let state = match results.get(&provider_job_id) {
                        Some(JobState::Completed(JobOutcome::Provider(status)))
                            if status.is_ready() =>
                        {
                            JobState::Completed(JobOutcome::ProviderPackage(
                                self.executor.install_provider_package(package, status),
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
                let previous = results.insert(job_id, state);
                debug_assert!(previous.is_none());
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
                        let previous =
                            results.insert(job_id, JobState::Completed(JobOutcome::Link(outcome)));
                        debug_assert!(previous.is_none());
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    for (job_id, _) in links {
                        let previous = results.insert(
                            job_id,
                            JobState::Blocked(BlockReason::LinkPhase {
                                message: message.clone(),
                            }),
                        );
                        debug_assert!(previous.is_none());
                    }
                    link_phase_error = Some(error);
                }
            }
        }

        JobExecutionReport {
            results,
            link_phase_error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::assert_link_projection;

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
}
