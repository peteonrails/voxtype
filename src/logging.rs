//! The log filter every voxtype binary starts with.

use tracing_subscriber::EnvFilter;

/// The filter `RUST_LOG` asks for, or `default` when it is unset, empty or
/// not a valid filter.
///
/// An empty `RUST_LOG` counts as unset. tracing-subscriber reads an empty
/// value as a filter with no directives, which turns off every log line,
/// warnings and errors included, so `RUST_LOG=` in a shell or a unit file
/// would silence voxtype instead of leaving its default alone. Nix's
/// `buildRustPackage` exports an empty `RUST_LOG` into every build, which
/// hid the warnings that the integration tests check for.
pub fn env_filter(default: &str) -> EnvFilter {
    filter_from(std::env::var("RUST_LOG").ok().as_deref(), default)
}

fn filter_from(rust_log: Option<&str>, default: &str) -> EnvFilter {
    rust_log
        .filter(|value| !value.trim().is_empty())
        .and_then(|value| EnvFilter::try_new(value).ok())
        .unwrap_or_else(|| EnvFilter::new(default))
}

#[cfg(test)]
mod tests {
    use super::filter_from;
    use tracing_subscriber::filter::LevelFilter;

    #[test]
    fn unset_or_empty_rust_log_keeps_the_default() {
        for rust_log in [None, Some(""), Some("  ")] {
            assert_eq!(
                filter_from(rust_log, "voxtype=info,warn").max_level_hint(),
                Some(LevelFilter::INFO),
                "{rust_log:?}"
            );
        }
    }

    #[test]
    fn a_valid_rust_log_wins_and_an_invalid_one_falls_back() {
        assert_eq!(
            filter_from(Some("error"), "voxtype=info,warn").max_level_hint(),
            Some(LevelFilter::ERROR)
        );
        assert_eq!(
            filter_from(Some("voxtype=notalevel"), "voxtype=info,warn").max_level_hint(),
            Some(LevelFilter::INFO)
        );
    }
}
