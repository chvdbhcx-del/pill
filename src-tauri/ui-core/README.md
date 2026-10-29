# pill-ui-core

Pure logic of the native Pill UI (no tauri, no windows crate): state machines, view-models, schemas, i18n.
Renderers in `src-tauri/src/ui/*` consume these. Test on its own: `cd src-tauri/ui-core && cargo test`.

Ownership (one agent per directory, nobody edits another area):

| dir | scope |
|---|---|
| shell | capsule state machine, reaction resolver, quiet hours, liquid edge sim, clock, placement |
| live | live-row providers as row specs (priority, ttl, accent) |
| panel | expanded-panel tab view-models |
| bar | command bar + uninstall state machine |
| settings | declarative settings schema, search, wizard |
| capture | screen capture selection / markup model |
| i18n | the ru/en string table |
