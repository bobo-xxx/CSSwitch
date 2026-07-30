#![allow(dead_code)]

use std::fmt;

use crate::config::GatewayConfig;
use crate::messages::{self, AttemptMode, OpenedInferenceResponse};
use crate::provider_failure::{
    AttemptController, AttemptDiagnostic, AttemptDirective, AttemptSnapshot, FailureObservation,
    ProviderFailure, RouteContext, TransitionError,
};

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct ApiKeyAttemptSequence;

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) trait ApiKeyTransport {
    type Opened;

    fn post_once(
        &mut self,
        body: &[u8],
        mode: AttemptMode,
    ) -> Result<Self::Opened, FailureObservation>;
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) trait AttemptRuntime {
    fn cancelled(&mut self) -> bool;
    fn wait(&mut self, delay_ms: u64) -> bool;
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum OpenResult<O> {
    Opened(OpenedAttempt<O>),
    Failed(TerminalAttempt),
    Cancelled(TerminalAttempt),
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct OpenedAttempt<O> {
    pub(crate) opened: O,
    controller: AttemptController,
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct TerminalAttempt {
    pub(crate) failure: Option<ProviderFailure>,
    controller: AttemptController,
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct ApiKeyPostOnceTransport<'a> {
    cfg: &'a GatewayConfig,
    attempt_mode: AttemptMode,
}

impl ApiKeyAttemptSequence {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn open<T>(
        context: RouteContext,
        body: &[u8],
        mode: AttemptMode,
        transport: &mut T,
        runtime: &mut impl AttemptRuntime,
    ) -> OpenResult<T::Opened>
    where
        T: ApiKeyTransport,
    {
        let mut controller = AttemptController::new(context.clone(), false);
        loop {
            if runtime.cancelled() {
                controller
                    .observe(FailureObservation::Cancelled)
                    .expect("pre-post API-key cancellation is terminal");
                return OpenResult::Cancelled(TerminalAttempt {
                    failure: None,
                    controller,
                });
            }
            controller
                .begin_post()
                .expect("controller authorized API-key POST");
            match transport.post_once(body, mode) {
                Ok(opened) => {
                    controller
                        .mark_response_started()
                        .expect("opened API-key response follows in-flight POST");
                    return OpenResult::Opened(OpenedAttempt { opened, controller });
                }
                Err(observation) => match controller
                    .observe(observation)
                    .expect("in-flight API-key observation is authorized")
                {
                    AttemptDirective::RetryAfter(delay_ms) => {
                        if !runtime.wait(delay_ms) {
                            controller
                                .observe(FailureObservation::Cancelled)
                                .expect("API-key cancellation during wait is terminal");
                            return OpenResult::Cancelled(TerminalAttempt {
                                failure: None,
                                controller,
                            });
                        }
                    }
                    AttemptDirective::Fail(failure) => {
                        return OpenResult::Failed(TerminalAttempt {
                            failure: Some(failure),
                            controller,
                        });
                    }
                    AttemptDirective::Cancel => {
                        return OpenResult::Cancelled(TerminalAttempt {
                            failure: None,
                            controller,
                        });
                    }
                    AttemptDirective::RepairOnce(_) => {
                        debug_assert!(false, "API-key attempt sequence never enables repair");
                        let failure = ProviderFailure::from_observation(
                            &context,
                            &FailureObservation::Protocol(
                                crate::provider_failure::ProtocolKind::InvalidResponse,
                            ),
                            false,
                        );
                        return OpenResult::Failed(TerminalAttempt {
                            failure: Some(failure),
                            controller,
                        });
                    }
                },
            }
        }
    }
}

impl<O> OpenedAttempt<O> {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn fail_after_open(mut self, observation: FailureObservation) -> TerminalAttempt {
        let directive = self
            .controller
            .observe(observation)
            .expect("opened API-key response failure is authorized");
        let AttemptDirective::Fail(failure) = directive else {
            unreachable!("opened response failure must be terminal")
        };
        TerminalAttempt {
            failure: Some(failure),
            controller: self.controller,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn completed_diagnostic(&mut self) -> Result<AttemptDiagnostic, TransitionError> {
        self.controller.completed_diagnostic()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn cancelled_diagnostic(&mut self) -> Result<AttemptDiagnostic, TransitionError> {
        self.controller.cancelled_diagnostic()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn snapshot(&self) -> AttemptSnapshot {
        self.controller.snapshot()
    }
}

impl TerminalAttempt {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn snapshot(&self) -> AttemptSnapshot {
        self.controller.snapshot()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn failed_diagnostic(&mut self) -> Result<AttemptDiagnostic, TransitionError> {
        let failure = self
            .failure
            .as_ref()
            .expect("failed diagnostic requires a provider failure");
        self.controller.failed_diagnostic(failure)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn cancelled_diagnostic(&mut self) -> Result<AttemptDiagnostic, TransitionError> {
        self.controller.cancelled_diagnostic()
    }
}

impl<'a> ApiKeyPostOnceTransport<'a> {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn new(cfg: &'a GatewayConfig, attempt_mode: AttemptMode) -> Self {
        Self { cfg, attempt_mode }
    }
}

impl ApiKeyTransport for ApiKeyPostOnceTransport<'_> {
    type Opened = OpenedInferenceResponse;

    fn post_once(
        &mut self,
        body: &[u8],
        mode: AttemptMode,
    ) -> Result<Self::Opened, FailureObservation> {
        debug_assert_eq!(
            mode, self.attempt_mode,
            "transport adapter mode must match sequence mode"
        );
        messages::post_once(self.cfg, body, self.attempt_mode)
    }
}

impl fmt::Debug for ApiKeyPostOnceTransport<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiKeyPostOnceTransport")
            .field("provider", &self.cfg.provider)
            .field("attempt_mode", &self.attempt_mode)
            .finish()
    }
}

#[cfg(test)]
mod tests;
