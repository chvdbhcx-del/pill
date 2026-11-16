# Share your look (`look.share`, Look page)

The island's APPEARANCE as one small file or one line of text, and the way back in. The pure rules live in `ui-core/src/settings/look.rs` (unit tested on their
own); the window half is `src/ui/settings_view/look.rs`; the only backend command is `look_export` (`settings_backup.rs`).

## What the user sees

Look > "Share your look" (row `look.share`, a custom widget):

| piece | does |
|---|---|
| chips Midnight / Paper / Neon / Quiet | the four built-in looks; they go through the same parse / validate / preview door as anything pasted |
| Save file | writes `<name>.pilllook` into Downloads (never over an older file: `name (2).pilllook`) and selects it in Explorer |
| Copy as text | `pill-look:` + base64url of the compact JSON on the clipboard: one line that goes through any chat |
| Open file | the system dialog (`*.pilllook`, `*.json`), at most 8 KiB |
| Paste from clipboard | accepts the `pill-look:` text, or a pasted `.pilllook` JSON |

A look that is staged (any of the three import ways, or a chip) is **pending**: it is validated into a `Plan`, shown in the live preview (the preview island is fed
`model values + plan` through `LookState::overlay`, which costs nothing when nothing is pending) and a banner opens under the buttons with Apply and Cancel.
Nothing is written yet. **Apply** goes through `SettingsModel::try_set` for every key of the plan (so the window and the preview agree at once), sends ONE
`settings_set`, and an animated toast at the bottom of the page offers **Undo** for 12 s; Undo writes the previous values back (again one patch). **Cancel**
puts the preview back (the keys the look touched are handed to it once, then the overlay is a plain borrow again). A look that would change nothing says so
instead of opening a banner. The banner, the toast and the chips animate with `Flow` / `Feel`; Reduce motion lands every value at once; with no look in play
nothing runs (the toast's expiry is one sleeping thread per Apply that posts a wake-up).

## The file: schema version 1

```json
{
  "kind": "pill-look",
  "version": 1,
  "name": "Midnight",
  "author": "optional, at most 40 characters",
  "values": { "theme": "dark", "island.color": "#0b1b3d", "size": 100, "pet": "ghost" }
}
```

* `kind` must be `pill-look`; `version` an integer >= 1. A newer version than the app knows is refused ("made by a newer Pill"); unknown top-level fields are
  ignored (forward compatibility). `name` / `author` are optional strings, control characters stripped, 40 characters at most. The app never writes an author
  by itself (nothing identifying); a hand-written one is kept.
* `values` holds a subset of the whitelist below. A look may carry only some keys; absent keys are left alone. Export always writes all of them.
* The text form is `pill-look:` + base64url (no padding; `+` `/` and `=` are tolerated on input) of the compact JSON, at most 4096 characters; whitespace and
  line breaks that a chat inserts are ignored. A file is at most 8 KiB.

## The whitelist rule

`look::WHITELIST` is the ONLY set of keys that is ever exported or accepted. It is appearance, nothing else:

| group | keys |
|---|---|
| the island | `theme`, `island.color`, `island.colorCustom`, `shape`, `size`, `opacityCol`, `opacityExp` |
| where it stands | `edge.position`, `edge.align`, `underTaskbar`, `offsetX` |
| clock and date | `showTime`, `showDate`, `showSeconds`, `clock24` |
| edge style and pet | `character`, `character.liquid`, `pet`, `pet.side`, `pet.size` |
| the player's look and place | `musicPill`, `home.player`, `media.widget.edge`, `media.widget.align`, `media.widget.offset` |

Never part of a look: `media.widget` itself (it switches a window and a track read on or off: a privacy gate), `module.*`, `live.*`, `reduceMotion` (an
accessibility need, not a taste), `language`, `monitor`, `fps` / `ui.gpu` (the PC's own business), hotkeys, `ai.*` / `voice.*` / `net.*` / `weather.*` /
`mail.*` / `phone.*` / calendar / relay settings, tokens, DPAPI blobs, folders and paths, anything that names the person or the PC.

The rule is enforced by tests: the list is frozen in `look.rs::tests::FROZEN` (adding a key means editing it on purpose), every key must be a stored schema row,
free text is only the colour, no key may start with a forbidden prefix, an export of a settings model full of strangers contains only whitelisted keys, and an
import with any one stranger is refused as a whole.

## Import validation (strict, in this order)

1. size (file 8 KiB, text 4096 characters), JSON / base64 shape, `kind`, `version` (`LookError::NotALook` / `Newer` / `TooBig` / `Empty`);
2. every key of `values` must be whitelisted, else the WHOLE look is refused (`Forbidden(key)`; the key is quoted without control characters);
3. every value goes through `SettingsModel::normalize`, the validator a click uses: options must be options, types must match, sliders are clamped and snapped
   (`size: 500` becomes 200, `opacityExp: 33` becomes 35), pets and edge styles must exist; `island.colorCustom` must be empty or a hex colour (`Bad(key)`);
4. no value at all: `NoValues`. Messages are in Russian and English (`LookError::message`).

Values are applied in whitelist order (`edge.position` before `edge.align`: picking the taskbar moves a centred side to the left, the look's own side must win).

## Tests

`cargo test -p pill-ui-core settings::look` (whitelist frozen, export whitelist, import refusals and validation, file / text roundtrip, base64url, apply and
undo, the taskbar side effect, the four built-in looks, messages); `settings_view::tests_look::*` (preview before write, Cancel, Apply is one patch, Undo,
refused pastes, PNGs); `settings_backup::tests::a_shared_look_never_names_a_path`.

## Not verified live, known gaps

* The real file dialog, the Explorer selection and a clipboard round trip with a real chat were not driven by hand.
* The exported name is always "My look" / "Моё оформление" (no name field yet; edit `name` in the file). `author` is read, never written.
* The live preview does not show `underTaskbar`, `offsetX` or the player widget's keys (the preview blocks them on purpose); a look carrying them still applies.
