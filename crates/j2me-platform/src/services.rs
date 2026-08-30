//! Explicit host seams for Java ME's external connection services.
//!
//! Old games commonly probe GCF HTTP or JSR-120 SMS. Device support and host
//! permission are two separate facts: [`ConnectorFragment`] records what the
//! selected phone exposed, while a caller-supplied [`ServiceBackend`] is the
//! only code allowed to perform an external action. The default backend denies
//! everything, so merely recovering a game can never replay an obsolete
//! telemetry request or send a message.

use std::collections::BTreeMap;

use j2me_device::ConnectorFragment;

use crate::PlatformError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub url: String,
    pub method: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn new(url: impl Into<String>, method: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: method.into(),
            headers: BTreeMap::new(),
            body: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmsRequest {
    /// Exact GCF connection URL, normally `sms://<number>`.
    pub connection_url: String,
    pub payload: String,
}

pub trait ServiceBackend {
    fn execute_http(&mut self, request: &HttpRequest) -> Result<HttpResponse, PlatformError>;

    fn send_sms(&mut self, request: &SmsRequest) -> Result<(), PlatformError>;
}

/// Safe backend for preservation tools, tests, and hosts that have not exposed
/// an explicit user-controlled external-services policy.
#[derive(Debug, Default, Clone, Copy)]
pub struct DisabledServices;

impl ServiceBackend for DisabledServices {
    fn execute_http(&mut self, _request: &HttpRequest) -> Result<HttpResponse, PlatformError> {
        Err(PlatformError::Service(
            "HTTP backend is disabled by host policy".to_owned(),
        ))
    }

    fn send_sms(&mut self, _request: &SmsRequest) -> Result<(), PlatformError> {
        Err(PlatformError::Service(
            "SMS backend is disabled by host policy".to_owned(),
        ))
    }
}

pub struct ServiceRuntime<B> {
    capabilities: ConnectorFragment,
    backend: B,
}

impl<B: ServiceBackend> ServiceRuntime<B> {
    pub fn new(capabilities: ConnectorFragment, backend: B) -> Self {
        Self {
            capabilities,
            backend,
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn execute_http(&mut self, request: &HttpRequest) -> Result<HttpResponse, PlatformError> {
        let scheme = connection_scheme(&request.url)?;
        if !matches!(scheme.as_str(), "http" | "https") {
            return Err(PlatformError::Service(format!(
                "{scheme:?} is not an HttpConnection scheme"
            )));
        }
        self.require_capability(&scheme)?;
        if !matches!(request.method.as_str(), "GET" | "POST" | "HEAD") {
            return Err(PlatformError::Service(format!(
                "unsupported HttpConnection request method {:?}",
                request.method
            )));
        }
        self.backend.execute_http(request)
    }

    pub fn send_sms(&mut self, request: &SmsRequest) -> Result<(), PlatformError> {
        let scheme = connection_scheme(&request.connection_url)?;
        if scheme != "sms" {
            return Err(PlatformError::Service(format!(
                "{scheme:?} is not a MessageConnection SMS scheme"
            )));
        }
        self.require_capability(&scheme)?;
        self.backend.send_sms(request)
    }

    fn require_capability(&self, scheme: &str) -> Result<(), PlatformError> {
        if self.capabilities.supports(scheme) {
            Ok(())
        } else {
            Err(PlatformError::Service(format!(
                "selected device profile does not expose Connector scheme {scheme:?}"
            )))
        }
    }
}

fn connection_scheme(url: &str) -> Result<String, PlatformError> {
    let (scheme, _) = url.split_once(':').ok_or_else(|| {
        PlatformError::Service(format!("connection name has no URI scheme: {url:?}"))
    })?;
    if scheme.is_empty()
        || !scheme.bytes().all(|byte| {
            byte.is_ascii_alphabetic()
                || byte.is_ascii_digit()
                || matches!(byte, b'+' | b'-' | b'.')
        })
    {
        return Err(PlatformError::Service(format!(
            "connection name has an invalid URI scheme: {url:?}"
        )));
    }
    Ok(scheme.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[derive(Debug, Default)]
    struct RecordingBackend {
        http: Vec<HttpRequest>,
        sms: Vec<SmsRequest>,
    }

    impl ServiceBackend for RecordingBackend {
        fn execute_http(&mut self, request: &HttpRequest) -> Result<HttpResponse, PlatformError> {
            self.http.push(request.clone());
            Ok(HttpResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: b"fixture".to_vec(),
            })
        }

        fn send_sms(&mut self, request: &SmsRequest) -> Result<(), PlatformError> {
            self.sms.push(request.clone());
            Ok(())
        }
    }

    fn capabilities(schemes: &[&str]) -> ConnectorFragment {
        ConnectorFragment {
            schemes: schemes
                .iter()
                .map(|scheme| (*scheme).to_owned())
                .collect::<BTreeSet<_>>(),
        }
    }

    #[test]
    fn profile_and_host_permission_are_independent_gates() {
        let request = HttpRequest::new("http://example.invalid/scores", "POST");
        let mut unsupported = ServiceRuntime::new(capabilities(&[]), RecordingBackend::default());
        assert!(unsupported.execute_http(&request).is_err());
        assert!(unsupported.backend().http.is_empty());

        let mut disabled = ServiceRuntime::new(capabilities(&["http"]), DisabledServices);
        assert!(disabled.execute_http(&request).is_err());

        let mut enabled =
            ServiceRuntime::new(capabilities(&["http", "sms"]), RecordingBackend::default());
        assert_eq!(enabled.execute_http(&request).unwrap().body, b"fixture");
        enabled
            .send_sms(&SmsRequest {
                connection_url: "sms://12345".to_owned(),
                payload: "fixture".to_owned(),
            })
            .unwrap();
        assert_eq!(enabled.backend().http, vec![request]);
        assert_eq!(enabled.backend().sms.len(), 1);
    }

    #[test]
    fn wrong_schemes_and_methods_fail_before_the_backend() {
        let mut runtime =
            ServiceRuntime::new(capabilities(&["http", "sms"]), RecordingBackend::default());
        assert!(runtime
            .execute_http(&HttpRequest::new("sms://123", "POST"))
            .is_err());
        assert!(runtime
            .execute_http(&HttpRequest::new("http://example.invalid", "PATCH"))
            .is_err());
        assert!(runtime
            .send_sms(&SmsRequest {
                connection_url: "http://example.invalid".to_owned(),
                payload: String::new(),
            })
            .is_err());
        assert!(runtime.backend().http.is_empty());
        assert!(runtime.backend().sms.is_empty());
    }
}
