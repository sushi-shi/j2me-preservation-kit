# j2me-device

Composable descriptions of the handset a Java ME build targeted. The crate
keeps display, input, media, haptics, RMS, Connector protocols, system
environment, font, and lifecycle behavior on separate axes: two builds may
share a key layout without sharing an audio implementation, default charset,
system-property defaults, network API, or canvas size.

Concrete profiles are evidence owned by each game in `device-profiles.toml`.
This crate supplies the schema, validation, and resolver; it deliberately does
not turn observations from one game into universal vendor presets.

`device-profiles.toml` has named fragment tables on nine axes and composes them
with `[[profile]]` rows:

```toml
schema_version = 1

[display.reviewed_240x320]
width = 240
height = 320

# [input.*], [media.*], [haptics.*], [rms.*], [connector.*], [system.*],
# [font.*], [lifecycle.*]
# are filled from this game's evidence.

[[profile]]
id = "build-family-a"
display = "reviewed_240x320"
input = "..."
media = "..."
haptics = "..."
rms = "..."
connector = "..."
system = "..."
font = "..."
lifecycle = "..."
```

Every JAR row in `java/reconstruction/builds.toml` names one profile. Concrete
Nokia S60 keypad behavior is opt-in through `j2me-device-nokia`; the Nokia UI
extension API remains separately opt-in through `j2me-nokia`.

Pointer, pointer-motion, and key-repeat callback support are explicit booleans
on the input fragment. GCF schemes such as `http`, `https`, and `sms` are listed
on the connector fragment. Those are handset facts only: `j2me-platform` still
requires a host backend, and its default backend denies every external action.

The system fragment records the handset's default charset and the default map
seen by `System.getProperty`. Dynamic host/session/operator overrides stay in a
separate `j2me-platform::SystemEnvironment` layer, so an override never becomes
false device evidence. Charset names are evidence, not guesses: the fragment
does not claim that every host implements every handset encoding.
