//! Capability-aware native vibration seam.

pub trait VibrationEndpoint {
    fn vibrate(&mut self, duration_ms: u32) -> Result<(), String>;
}

#[derive(Debug, Default)]
pub struct UnavailableVibrator;

impl VibrationEndpoint for UnavailableVibrator {
    fn vibrate(&mut self, _: u32) -> Result<(), String> {
        Err("no native vibration endpoint is installed".to_owned())
    }
}

#[derive(Debug)]
pub struct NativeHaptics<V> {
    supported_by_profile: bool,
    endpoint: V,
}

impl<V: VibrationEndpoint> NativeHaptics<V> {
    pub fn for_profile(profile: &j2me_device::HapticsFragment, endpoint: V) -> Self {
        Self {
            supported_by_profile: profile.vibration,
            endpoint,
        }
    }

    pub fn vibrate(&mut self, duration_ms: i32) -> Result<bool, String> {
        if duration_ms < 0 {
            return Err("negative vibration duration".to_owned());
        }
        if !self.supported_by_profile || duration_ms == 0 {
            return Ok(false);
        }
        self.endpoint.vibrate(duration_ms as u32)?;
        Ok(true)
    }
}
