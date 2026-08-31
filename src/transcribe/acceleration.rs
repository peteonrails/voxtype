//! Acceleration information reported by a loaded transcriber.

/// Provider registration observed while loading a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRegistration {
    /// Every session rejected the requested provider and loaded on CPU.
    CpuFallback,
    /// At least one session registered the provider; execution remains unverified.
    RegisteredOrMixed,
}

/// A loaded model's provider registration result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccelerationStatus {
    /// The requested execution provider.
    pub backend: &'static str,
    /// The aggregate result across the model's sessions.
    pub registration: ProviderRegistration,
}

#[cfg(feature = "parakeet-webgpu")]
impl AccelerationStatus {
    pub(crate) fn combined_with(self, other: Self) -> Option<Self> {
        if self.backend != other.backend {
            return None;
        }

        let registration = match (self.registration, other.registration) {
            (ProviderRegistration::CpuFallback, ProviderRegistration::CpuFallback) => {
                ProviderRegistration::CpuFallback
            }
            _ => ProviderRegistration::RegisteredOrMixed,
        };
        Some(Self {
            backend: self.backend,
            registration,
        })
    }
}

#[cfg(all(test, feature = "parakeet-webgpu"))]
mod tests {
    use super::{AccelerationStatus, ProviderRegistration};

    #[test]
    fn concurrent_models_require_every_session_to_fall_back_before_reporting_cpu() {
        use ProviderRegistration::{CpuFallback, RegisteredOrMixed};

        let cases = [
            (CpuFallback, CpuFallback, CpuFallback),
            (CpuFallback, RegisteredOrMixed, RegisteredOrMixed),
            (RegisteredOrMixed, CpuFallback, RegisteredOrMixed),
            (RegisteredOrMixed, RegisteredOrMixed, RegisteredOrMixed),
        ];
        for (dictation, meeting, expected) in cases {
            assert_eq!(
                AccelerationStatus {
                    backend: "webgpu",
                    registration: dictation,
                }
                .combined_with(AccelerationStatus {
                    backend: "webgpu",
                    registration: meeting,
                }),
                Some(AccelerationStatus {
                    backend: "webgpu",
                    registration: expected,
                })
            );
        }
    }
}
