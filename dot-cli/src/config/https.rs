use std::io::{self, Read};
use std::string::FromUtf8Error;

use url::Url;

const MAX_REDIRECTS: u32 = 10;

pub(crate) fn fetch(url: &Url) -> Result<String, HttpsError> {
    let agent = build_agent();
    let mut response = agent
        .get(url.as_str())
        .call()
        .map_err(HttpsError::from_call)?;
    validate_final_status(response.status().as_u16())?;
    read_body(response.body_mut().as_reader())
}

fn build_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(MAX_REDIRECTS)
        .max_redirects_will_error(true)
        .http_status_as_error(true)
        .build()
        .into()
}

fn validate_final_status(status: u16) -> Result<(), HttpsError> {
    if (200..=299).contains(&status) {
        Ok(())
    } else {
        Err(HttpsError::StatusCode {
            status,
            source: None,
        })
    }
}

fn read_body(mut reader: impl Read) -> Result<String, HttpsError> {
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|source| HttpsError::BodyRead { source })?;
    String::from_utf8(bytes).map_err(|source| HttpsError::InvalidUtf8 { source })
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum HttpsError {
    #[error("HTTPS-only policy rejected the request or an insecure redirect: {source}")]
    RequireHttpsOnly {
        #[source]
        source: ureq::Error,
    },
    #[error("HTTPS redirect limit of {MAX_REDIRECTS} was exhausted: {source}")]
    TooManyRedirects {
        #[source]
        source: ureq::Error,
    },
    #[error("HTTP response status {status} is not successful")]
    StatusCode {
        status: u16,
        #[source]
        source: Option<ureq::Error>,
    },
    #[error("HTTPS transport failed: {source}")]
    TransportIo {
        #[source]
        source: io::Error,
    },
    #[error("HTTPS transport failed: {source}")]
    Transport {
        #[source]
        source: ureq::Error,
    },
    #[error("failed to read the HTTPS response body: {source}")]
    BodyRead {
        #[source]
        source: io::Error,
    },
    #[error("HTTPS response body is not valid UTF-8: {source}")]
    InvalidUtf8 {
        #[source]
        source: FromUtf8Error,
    },
}

impl HttpsError {
    fn from_call(source: ureq::Error) -> Self {
        match source {
            source @ ureq::Error::RequireHttpsOnly(_) => Self::RequireHttpsOnly { source },
            source @ ureq::Error::TooManyRedirects => Self::TooManyRedirects { source },
            source @ ureq::Error::StatusCode(status) => Self::StatusCode {
                status,
                source: Some(source),
            },
            ureq::Error::Io(source) => Self::TransportIo { source },
            source => Self::Transport { source },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::io::{self, Cursor, Read};

    use super::*;

    #[test]
    fn agent_enforces_https_redirect_and_status_policy() {
        let agent = build_agent();
        let config = agent.config();

        assert!(config.https_only());
        assert_eq!(config.max_redirects(), 10);
        assert!(config.max_redirects_will_error());
        assert!(config.http_status_as_error());
    }

    #[test]
    fn maps_each_ureq_call_error_category() {
        let require_https = HttpsError::from_call(ureq::Error::RequireHttpsOnly(
            "http://example.com/dot.toml".to_owned(),
        ));
        assert!(matches!(require_https, HttpsError::RequireHttpsOnly { .. }));
        assert!(Error::source(&require_https).is_some_and(|source| source.is::<ureq::Error>()));

        let redirects = HttpsError::from_call(ureq::Error::TooManyRedirects);
        assert!(matches!(redirects, HttpsError::TooManyRedirects { .. }));

        let status = HttpsError::from_call(ureq::Error::StatusCode(503));
        assert!(matches!(
            status,
            HttpsError::StatusCode {
                status: 503,
                source: Some(_),
            }
        ));

        let transport = HttpsError::from_call(ureq::Error::ConnectionFailed);
        assert!(matches!(transport, HttpsError::Transport { .. }));
        assert!(Error::source(&transport).is_some_and(|source| source.is::<ureq::Error>()));
    }

    #[test]
    fn maps_ureq_io_to_the_underlying_typed_transport_source() {
        let error = HttpsError::from_call(ureq::Error::Io(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "network stopped",
        )));

        let source = Error::source(&error)
            .and_then(|source| source.downcast_ref::<io::Error>())
            .expect("transport I/O should be the immediate typed source");
        assert_eq!(source.kind(), io::ErrorKind::ConnectionAborted);
    }

    #[test]
    fn final_status_accepts_only_success_responses() {
        for status in [200, 204, 299] {
            validate_final_status(status).expect("2xx status should succeed");
        }
        for status in [300, 302, 399] {
            assert!(matches!(
                validate_final_status(status),
                Err(HttpsError::StatusCode {
                    status: actual,
                    source: None,
                }) if actual == status
            ));
        }
    }

    #[test]
    fn body_decoding_preserves_io_and_utf8_failures() {
        struct FailingReader;

        impl Read for FailingReader {
            fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("response body stopped"))
            }
        }

        let read = read_body(FailingReader).expect_err("body I/O should fail");
        assert!(matches!(read, HttpsError::BodyRead { .. }));
        assert!(Error::source(&read).is_some_and(|source| source.is::<io::Error>()));

        let utf8 = read_body(Cursor::new([0xff])).expect_err("invalid UTF-8 should fail");
        assert!(matches!(utf8, HttpsError::InvalidUtf8 { .. }));
        assert!(
            Error::source(&utf8).is_some_and(|source| source.is::<std::string::FromUtf8Error>())
        );
    }
}
