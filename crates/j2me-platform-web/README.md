# j2me-platform-web

Browser-only adapters for Java ME ports. WebAudio, gesture resume, browser
input, and `navigator.vibrate` live here so native and core crates remain free
of Web APIs.

The adapter decodes the same shared AMR/MIDI/MMF/WAV formats into WebAudio
buffers, resumes an autoplay-suspended context only from a user gesture, and
uses reflection around `navigator.vibrate` so a missing API cannot trap WASM.
