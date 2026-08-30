//! Linux evdev force-feedback endpoint for MIDP `Display.vibrate`.
//!
//! A supported phone profile permits the request; this endpoint then drives an
//! `FF_RUMBLE` device when available and reports a concrete refusal otherwise.
//! `J2ME_VIBRATION_DEVICE` may pin one `/dev/input/event*` node.

pub struct EvdevVibrator {
    device: Option<platform::Device>,
    endpoint: String,
    refusal: Option<String>,
}

impl EvdevVibrator {
    /// Probe without turning absence, permissions, or unsupported hardware into
    /// a panic. Every unavailable result retains a human-readable reason.
    pub fn open() -> Self {
        match platform::open() {
            Ok((device, endpoint)) => Self {
                device: Some(device),
                endpoint,
                refusal: None,
            },
            Err(refusal) => Self {
                device: None,
                endpoint: "none".to_owned(),
                refusal: Some(refusal),
            },
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    /// Upload a full-magnitude rumble effect whose kernel-managed lifetime is
    /// exactly the requested duration. MIDP supplies no intensity or waveform.
    pub fn play(&mut self, duration_ms: u32) -> Result<(), String> {
        match self.device.as_mut() {
            Some(device) => device.play(duration_ms),
            None => Err(self
                .refusal
                .clone()
                .unwrap_or_else(|| "no vibration device".to_owned())),
        }
    }
}

impl crate::haptics::VibrationEndpoint for EvdevVibrator {
    fn vibrate(&mut self, duration_ms: u32) -> Result<(), String> {
        self.play(duration_ms)
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    pub struct Device;

    impl Device {
        pub fn play(&mut self, _: u32) -> Result<(), String> {
            Err("only Linux evdev force feedback is implemented".to_owned())
        }
    }

    pub fn open() -> Result<(Device, String), String> {
        Err("only Linux evdev force feedback is implemented".to_owned())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    const EV_FF: u16 = 0x15;
    const FF_RUMBLE: u16 = 0x50;
    const IOC_READ: u64 = 2;
    const IOC_WRITE: u64 = 1;
    const IOC_DIRSHIFT: u64 = 30;
    const IOC_SIZESHIFT: u64 = 16;
    const IOC_TYPESHIFT: u64 = 8;

    const fn ioc(direction: u64, kind: u64, number: u64, size: u64) -> u64 {
        (direction << IOC_DIRSHIFT) | (size << IOC_SIZESHIFT) | (kind << IOC_TYPESHIFT) | number
    }

    pub(super) fn eviocgbit_ff(length: u64) -> u64 {
        ioc(IOC_READ, b'E' as u64, 0x20 + EV_FF as u64, length)
    }

    pub(super) fn eviocsff() -> u64 {
        ioc(
            IOC_WRITE,
            b'E' as u64,
            0x80,
            std::mem::size_of::<FfEffect>() as u64,
        )
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    pub(super) struct FfReplay {
        length: u16,
        delay: u16,
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    pub(super) struct FfTrigger {
        button: u16,
        interval: u16,
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    pub(super) struct FfRumbleEffect {
        strong_magnitude: u16,
        weak_magnitude: u16,
    }

    /// Linux `struct ff_effect`, with the union represented by its rumble arm
    /// plus padding to the ABI's widest periodic-effect member.
    #[repr(C, align(8))]
    #[derive(Debug, Clone, Copy, Default)]
    pub(super) struct FfEffect {
        kind: u16,
        id: i16,
        direction: u16,
        trigger: FfTrigger,
        replay: FfReplay,
        union_alignment: [u8; 2],
        pub(super) rumble: FfRumbleEffect,
        tail_padding: [u8; 28],
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    pub(super) struct InputEvent {
        seconds: libc::time_t,
        microseconds: libc::suseconds_t,
        kind: u16,
        code: u16,
        pub(super) value: i32,
    }

    pub struct Device {
        path: String,
        file: OwnedFd,
        effect_id: i16,
    }

    pub fn open() -> Result<(Device, String), String> {
        if let Ok(pinned) = std::env::var("J2ME_VIBRATION_DEVICE") {
            return open_path(&pinned)
                .map(|device| {
                    let endpoint = endpoint_of(&device);
                    (device, endpoint)
                })
                .map_err(|error| format!("J2ME_VIBRATION_DEVICE={pinned}: {error}"));
        }
        let entries =
            std::fs::read_dir("/dev/input").map_err(|error| format!("/dev/input: {error}"))?;
        let mut candidates: Vec<String> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("event"))
            })
            .filter_map(|path| path.to_str().map(ToOwned::to_owned))
            .collect();
        candidates.sort();
        if candidates.is_empty() {
            return Err("no /dev/input/event* nodes exist".to_owned());
        }
        let mut last = "no /dev/input/event* node supports FF_RUMBLE".to_owned();
        for path in candidates {
            match open_path(&path) {
                Ok(device) => {
                    let endpoint = endpoint_of(&device);
                    return Ok((device, endpoint));
                }
                Err(error) if error.contains("Permission denied") => {
                    last = format!("{path}: {error}");
                }
                Err(_) => {}
            }
        }
        Err(last)
    }

    fn endpoint_of(device: &Device) -> String {
        format!("{} (evdev FF_RUMBLE)", device.path)
    }

    fn open_path(path: &str) -> Result<Device, String> {
        let name = CString::new(path).map_err(|error| error.to_string())?;
        // SAFETY: `name` is a live, NUL-terminated path and the returned fd is
        // immediately transferred into `OwnedFd`.
        let raw = unsafe { libc::open(name.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if raw < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // SAFETY: `raw` is a fresh, exclusively owned descriptor.
        let file = unsafe { OwnedFd::from_raw_fd(raw) };
        let mut effects = [0_u8; (FF_RUMBLE as usize / 8) + 1];
        // SAFETY: the ioctl request encodes the exact live output-buffer size.
        let read = unsafe {
            libc::ioctl(
                file.as_raw_fd(),
                eviocgbit_ff(effects.len() as u64),
                effects.as_mut_ptr(),
            )
        };
        if read < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let byte = FF_RUMBLE as usize / 8;
        let bit = u32::from(FF_RUMBLE) % 8;
        if effects.get(byte).copied().unwrap_or(0) & (1 << bit) == 0 {
            return Err("no FF_RUMBLE effect".to_owned());
        }
        Ok(Device {
            path: path.to_owned(),
            file,
            effect_id: -1,
        })
    }

    impl Device {
        pub fn play(&mut self, duration_ms: u32) -> Result<(), String> {
            let mut effect = FfEffect {
                kind: FF_RUMBLE,
                id: self.effect_id,
                direction: 0,
                trigger: FfTrigger::default(),
                replay: FfReplay {
                    length: duration_ms.min(u32::from(u16::MAX)) as u16,
                    delay: 0,
                },
                union_alignment: [0; 2],
                rumble: FfRumbleEffect {
                    strong_magnitude: u16::MAX,
                    weak_magnitude: u16::MAX,
                },
                tail_padding: [0; 28],
            };
            // SAFETY: the descriptor is an evdev node and `effect` is pinned
            // for the call with the kernel ABI layout tested below.
            let uploaded = unsafe {
                libc::ioctl(
                    self.file.as_raw_fd(),
                    eviocsff(),
                    std::ptr::addr_of_mut!(effect),
                )
            };
            if uploaded < 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            self.effect_id = effect.id;
            let event = InputEvent {
                seconds: 0,
                microseconds: 0,
                kind: EV_FF,
                code: effect.id as u16,
                value: 1,
            };
            // SAFETY: write exactly one initialized Linux input event.
            let written = unsafe {
                libc::write(
                    self.file.as_raw_fd(),
                    std::ptr::addr_of!(event).cast(),
                    std::mem::size_of::<InputEvent>(),
                )
            };
            if written != std::mem::size_of::<InputEvent>() as isize {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(())
        }
    }
}

#[cfg(all(test, target_os = "linux", target_pointer_width = "64"))]
mod abi_tests {
    use super::platform::*;

    #[test]
    fn force_feedback_structures_match_the_64_bit_kernel_abi() {
        assert_eq!(std::mem::size_of::<FfReplay>(), 4);
        assert_eq!(std::mem::size_of::<FfTrigger>(), 4);
        assert_eq!(std::mem::size_of::<FfRumbleEffect>(), 4);
        assert_eq!(std::mem::size_of::<FfEffect>(), 48);
        assert_eq!(std::mem::align_of::<FfEffect>(), 8);
        assert_eq!(std::mem::offset_of!(FfEffect, rumble), 16);
        assert_eq!(std::mem::size_of::<InputEvent>(), 24);
        assert_eq!(std::mem::offset_of!(InputEvent, value), 20);
        assert_eq!(eviocsff(), 0x4030_4580);
        assert_eq!(eviocgbit_ff(9), 0x8009_4535);
    }
}
