use crate::PlatformError;
use j2me_device::SystemFragment;
use std::collections::BTreeMap;

/// Dynamic host/session/operator values layered over reviewed handset facts.
///
/// `None` is an explicit removal: it makes a device-default property absent.
/// This is intentionally separate from [`SystemFragment`] so a test harness or
/// host configuration cannot silently become handset evidence.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SystemOverrides {
    default_charset: Option<String>,
    properties: BTreeMap<String, Option<String>>,
}

impl SystemOverrides {
    pub fn set_default_charset(&mut self, charset: impl Into<String>) -> Result<(), PlatformError> {
        let charset = charset.into();
        if charset.trim().is_empty() {
            return Err(PlatformError::Config(
                "system default-charset override must not be empty".to_string(),
            ));
        }
        self.default_charset = Some(charset);
        Ok(())
    }

    pub fn clear_default_charset(&mut self) {
        self.default_charset = None;
    }

    pub fn set_property(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), PlatformError> {
        let name = checked_property_name(name.into())?;
        self.properties.insert(name, Some(value.into()));
        Ok(())
    }

    pub fn remove_property(&mut self, name: impl Into<String>) -> Result<(), PlatformError> {
        let name = checked_property_name(name.into())?;
        self.properties.insert(name, None);
        Ok(())
    }

    pub fn default_charset(&self) -> Option<&str> {
        self.default_charset.as_deref()
    }

    pub fn property(&self, name: &str) -> Option<Option<&str>> {
        self.properties.get(name).map(|value| value.as_deref())
    }
}

fn checked_property_name(name: String) -> Result<String, PlatformError> {
    if name.trim().is_empty() {
        Err(PlatformError::Config(
            "system-property override name must not be empty".to_string(),
        ))
    } else {
        Ok(name)
    }
}

/// Resolved Java ME system environment for one emulated handset session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemEnvironment {
    device_defaults: SystemFragment,
    overrides: SystemOverrides,
}

impl SystemEnvironment {
    pub fn from_device(device_defaults: &SystemFragment) -> Self {
        Self {
            device_defaults: device_defaults.clone(),
            overrides: SystemOverrides::default(),
        }
    }

    pub fn with_overrides(device_defaults: &SystemFragment, overrides: SystemOverrides) -> Self {
        Self {
            device_defaults: device_defaults.clone(),
            overrides,
        }
    }

    pub fn default_charset(&self) -> &str {
        self.overrides
            .default_charset()
            .unwrap_or(&self.device_defaults.default_charset)
    }

    /// Resolved `System.getProperty` value. An explicit host removal wins over
    /// the device default and is returned as `None`.
    pub fn property(&self, name: &str) -> Option<&str> {
        match self.overrides.property(name) {
            Some(value) => value,
            None => self.device_defaults.property(name),
        }
    }

    pub const fn device_defaults(&self) -> &SystemFragment {
        &self.device_defaults
    }

    pub const fn overrides(&self) -> &SystemOverrides {
        &self.overrides
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> SystemFragment {
        SystemFragment {
            default_charset: "ISO-8859-1".to_string(),
            properties: BTreeMap::from([
                (
                    "microedition.platform".to_string(),
                    "NokiaFixture".to_string(),
                ),
                (
                    "wireless.messaging.sms.smsc".to_string(),
                    "+111".to_string(),
                ),
            ]),
        }
    }

    #[test]
    fn device_defaults_are_used_without_overrides() {
        let environment = SystemEnvironment::from_device(&fixture());
        assert_eq!(environment.default_charset(), "ISO-8859-1");
        assert_eq!(
            environment.property("microedition.platform"),
            Some("NokiaFixture")
        );
        assert_eq!(environment.property("missing"), None);
    }

    #[test]
    fn session_values_override_or_remove_without_mutating_device_evidence() {
        let device = fixture();
        let mut overrides = SystemOverrides::default();
        overrides.set_default_charset("UTF-8").unwrap();
        overrides
            .set_property("wireless.messaging.sms.smsc", "+222")
            .unwrap();
        overrides.remove_property("microedition.platform").unwrap();
        let environment = SystemEnvironment::with_overrides(&device, overrides);

        assert_eq!(environment.default_charset(), "UTF-8");
        assert_eq!(
            environment.property("wireless.messaging.sms.smsc"),
            Some("+222")
        );
        assert_eq!(environment.property("microedition.platform"), None);
        assert_eq!(
            environment
                .device_defaults()
                .property("wireless.messaging.sms.smsc"),
            Some("+111")
        );
    }

    #[test]
    fn empty_override_names_and_charsets_fail() {
        let mut overrides = SystemOverrides::default();
        assert!(overrides.set_default_charset("  ").is_err());
        assert!(overrides.set_property("", "value").is_err());
        assert!(overrides.remove_property("\t").is_err());
    }
}
