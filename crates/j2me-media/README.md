# j2me-media

Platform-neutral audio formats and signal processing used by Java ME hosts:
AMR-NB storage parsing with a bit-exact MR122 decoder, SMF format 0/1 parsing,
SMAF/MMF parsing and writing, a documented SMAF-to-SMF approximation, narrow
PCM16 WAV decoding, a deterministic approximate MIDI renderer, and a
band-limited mono resampler.

Limitations are explicit. AMR speech modes other than MR122, SMF format 2 and
SMPTE timing, and Yamaha MA synthesis are rejected rather than silently
misrepresented. Phone capability and game playback policy do not live here.
