use super::acceleration::AccelerationStatus;
use parakeet_rs::ExecutionConfig;

#[cfg(feature = "parakeet-webgpu")]
use super::acceleration::ProviderRegistration;
#[cfg(feature = "parakeet-webgpu")]
use std::{cell::Cell, rc::Rc};

pub(super) struct ExecutionSetup {
    config: Option<ExecutionConfig>,
    #[cfg(feature = "parakeet-webgpu")]
    observations: Option<Rc<RegistrationObservations>>,
}

impl From<Option<ExecutionConfig>> for ExecutionSetup {
    fn from(config: Option<ExecutionConfig>) -> Self {
        Self {
            config,
            #[cfg(feature = "parakeet-webgpu")]
            observations: None,
        }
    }
}

impl ExecutionSetup {
    pub(super) fn load<T, E>(
        self,
        load: impl FnOnce(Option<ExecutionConfig>) -> Result<T, E>,
    ) -> Result<(T, Option<AccelerationStatus>), E> {
        let model = load(self.config)?;
        #[cfg(feature = "parakeet-webgpu")]
        let status = self
            .observations
            .and_then(|observations| observations.snapshot());
        #[cfg(not(feature = "parakeet-webgpu"))]
        let status = None;

        Ok((model, status))
    }

    #[cfg(feature = "parakeet-webgpu")]
    pub(super) fn webgpu() -> Self {
        use ort::ep::{ExecutionProvider, WebGPU, CPU};

        let observations = Rc::new(RegistrationObservations::default());
        let recorder = Rc::clone(&observations);
        let config = ExecutionConfig::new().with_custom_configure(move |builder| {
            let builder = builder.with_no_environment_execution_providers()?;
            let (builder, outcome) = register_with_cpu_fallback(
                builder,
                |builder| WebGPU::default().register(builder),
                |builder| CPU::default().register(builder),
            )?;
            recorder.record(outcome);
            Ok(builder)
        });

        Self {
            config: Some(config),
            observations: Some(observations),
        }
    }
}

#[cfg(feature = "parakeet-webgpu")]
#[derive(Default)]
struct RegistrationObservations {
    registration: Cell<Option<ProviderRegistration>>,
}

#[cfg(feature = "parakeet-webgpu")]
impl RegistrationObservations {
    fn record(&self, outcome: ProviderRegistration) {
        use ProviderRegistration::{CpuFallback, RegisteredOrMixed};

        let combined = match (self.registration.get(), outcome) {
            (Some(RegisteredOrMixed), _) | (_, RegisteredOrMixed) => RegisteredOrMixed,
            _ => CpuFallback,
        };
        self.registration.set(Some(combined));
    }

    fn snapshot(&self) -> Option<AccelerationStatus> {
        self.registration
            .get()
            .map(|registration| AccelerationStatus {
                backend: "webgpu",
                registration,
            })
    }
}

#[cfg(feature = "parakeet-webgpu")]
fn register_with_cpu_fallback<B: Clone, E: std::fmt::Display>(
    builder: B,
    register_gpu: impl FnOnce(&mut B) -> Result<(), E>,
    register_cpu: impl FnOnce(&mut B) -> Result<(), E>,
) -> Result<(B, ProviderRegistration), E> {
    let mut gpu_builder = builder.clone();
    match register_gpu(&mut gpu_builder) {
        Ok(()) => return Ok((gpu_builder, ProviderRegistration::RegisteredOrMixed)),
        Err(error) => tracing::warn!(%error, "WebGPU registration failed; using the CPU provider"),
    }

    let mut cpu_builder = builder;
    register_cpu(&mut cpu_builder)?;
    Ok((cpu_builder, ProviderRegistration::CpuFallback))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_load_errors_are_returned_without_cpu_retry() {
        let execution = ExecutionSetup::from(None);
        let actual = execution.load(|_| Err::<(), _>(std::io::ErrorKind::InvalidData));

        assert_eq!(actual, Err(std::io::ErrorKind::InvalidData));
    }

    #[cfg(feature = "parakeet-webgpu")]
    #[test]
    fn webgpu_registration_preserves_clean_options_for_cpu_fallback() {
        use ort::ep::RegisterError;

        for gpu_fails in [false, true] {
            let actual = register_with_cpu_fallback(
                vec!["original options"],
                |builder| {
                    builder.push("WebGPU options");
                    if gpu_fails {
                        return Err(RegisterError::MissingFeature);
                    }
                    Ok(())
                },
                |builder| {
                    builder.push("CPU options");
                    Ok(())
                },
            )
            .expect("provider configuration should succeed");
            let expected = if gpu_fails {
                (
                    vec!["original options", "CPU options"],
                    ProviderRegistration::CpuFallback,
                )
            } else {
                (
                    vec!["original options", "WebGPU options"],
                    ProviderRegistration::RegisteredOrMixed,
                )
            };

            assert_eq!(actual, expected);
        }
    }

    #[cfg(feature = "parakeet-webgpu")]
    #[test]
    fn cpu_registration_failure_is_returned_to_model_loading() {
        use ort::ep::RegisterError;

        let actual = register_with_cpu_fallback(
            (),
            |_| Err(RegisterError::MissingFeature),
            |_| Err(RegisterError::MissingFeature),
        );

        assert!(matches!(actual, Err(RegisterError::MissingFeature)));
    }

    #[cfg(feature = "parakeet-webgpu")]
    #[test]
    fn webgpu_session_registration_is_aggregated_without_claiming_gpu_use() {
        use ProviderRegistration::{CpuFallback, RegisteredOrMixed};

        let cases: &[(&[ProviderRegistration], Option<ProviderRegistration>)] = &[
            (&[], None),
            (&[CpuFallback, CpuFallback], Some(CpuFallback)),
            (
                &[RegisteredOrMixed, RegisteredOrMixed],
                Some(RegisteredOrMixed),
            ),
            (&[CpuFallback, RegisteredOrMixed], Some(RegisteredOrMixed)),
            (&[RegisteredOrMixed, CpuFallback], Some(RegisteredOrMixed)),
        ];

        for (outcomes, expected) in cases {
            let observations = RegistrationObservations::default();
            for outcome in *outcomes {
                observations.record(*outcome);
            }

            assert_eq!(
                observations.snapshot(),
                expected.map(|registration| AccelerationStatus {
                    backend: "webgpu",
                    registration,
                })
            );
        }
    }
}
