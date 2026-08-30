# j2me-device-nokia

Opt-in Nokia handset behavior that must remain visibly device-specific: the
Series 60 raw keypad codes and its `Canvas.getGameAction` mapping. It exposes an
input fragment, not a complete phone preset. A game owns the reviewed profile
composition in `device-profiles.toml` and should use this implementation only
when its evidence names the matching Nokia behavior.
