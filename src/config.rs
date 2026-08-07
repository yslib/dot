#![expect(
    clippy::result_large_err,
    reason = "typed configuration errors preserve precise context without boxing"
)]

use crate::schema::Config;
use crate::validation::{ConfigValidationError, validate_config};

/// An error parsing or statically validating in-memory configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigParseError {
    /// The source could not be deserialized as configuration.
    #[error("failed to deserialize configuration: {source}")]
    Deserialize {
        #[source]
        source: toml::de::Error,
    },
    /// The deserialized configuration failed static validation.
    #[error("configuration validation failed: {source}")]
    Validation {
        #[source]
        source: ConfigValidationError,
    },
}

impl Config {
    /// Parses and statically validates configuration from memory without filesystem, environment,
    /// platform, or runtime resolver evaluation.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigParseError::Deserialize`] for invalid TOML or configuration structure and
    /// [`ConfigParseError::Validation`] when static validation fails.
    pub fn parse(source: &str) -> Result<Self, ConfigParseError> {
        let config =
            toml::from_str(source).map_err(|source| ConfigParseError::Deserialize { source })?;
        validate_config(&config).map_err(|source| ConfigParseError::Validation { source })?;

        Ok(config)
    }
}
