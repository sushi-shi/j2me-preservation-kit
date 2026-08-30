# j2me-platform-native

Native-only adapters for Java ME ports: physical-key/profile projection, winit
focus conversion, a multi-player software mixer with CPAL output, and a
capability-aware pluggable vibration endpoint. Neither the device runtime nor
browser builds acquire desktop dependencies.

On Linux, `EvdevVibrator` probes `/dev/input/event*` for `FF_RUMBLE`; absence or
permissions are retained as an inspectable refusal. `J2ME_VIBRATION_DEVICE` can
pin a specific event node. Other platforms can install their own
`VibrationEndpoint` implementation without affecting MIDP semantics.
