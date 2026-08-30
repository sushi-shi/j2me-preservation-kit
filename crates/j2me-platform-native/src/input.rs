use j2me_device::InputFragment;
use j2me_input::{KeyCode, Keymap};

#[derive(Debug, Clone)]
pub struct NativeInput {
    keymap: Keymap,
    profile: InputFragment,
}

impl NativeInput {
    pub fn new(keymap: Keymap, profile: InputFragment) -> Self {
        Self { keymap, profile }
    }
    pub fn raw_code(&self, key: KeyCode) -> Option<i32> {
        self.keymap.raw_code(key, &self.profile)
    }
}
