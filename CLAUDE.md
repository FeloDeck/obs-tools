# CLAUDE.md

## Style

- Keep everything short: replies, explanations, comments, docs.
- Use simple English: short sentences, common words.
- Avoid em dashes (—). Do not just swap them for "-" either. Rewrite the sentence instead, for
  example with a comma, a colon, brackets or two sentences.

## Commits

- English only.
- Conventional Commits prefix: `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `ci:`, `build:`, `test:`.
- Subject as short as possible. Imperative, lowercase, no trailing period.
- Body only when something genuinely cannot be inferred from the diff.
- Never add `Co-Authored-By`, "Generated with" or any other AI attribution to commits or PR descriptions.
- Do not commit or push before I have checked the changes locally and said they are fine. Then
  commit and push. Several attempts at the same thing make one commit, and attempts that did not
  work make none. Different things done in one session get a commit each. This also goes for
  releases and tags.

## Project

- The plugin is plain JavaScript (ES modules) for Node.js 22+, without dependencies. Keep it that
  way: OpenDeck runs it with the system Node, and there is no build step.
- Plugin ID is `de.felitendo.obs-tools` (folder `de.felitendo.obs-tools.sdPlugin`), like the
  Feishin plugin. Do not change it again: OpenDeck keys profiles and settings by it.
- `manifest.json` and the icons in `images/` are generated. Change `scripts/manifest.mjs` or the
  glyphs in `plugin/lib/icons.js`, then run `make manifest` or `make icons`. `make icons` first
  measures every glyph (`scripts/fit-glyphs.mjs`, needs ImageMagick), so all icons end up the same
  optical size and optically centered (halfway between the middle of the box and the center of mass,
  so a play triangle does not look too far left). The action order in `scripts/manifest.mjs` is the order in OpenDeck:
  most used first.
- Settings pages come from `pi/schemas.js`. A new setting needs a default in the action class and
  a field in the schema. The status line is on top. Its "Change" button opens the connection
  settings right below it, and they open by themselves while OBS cannot be reached. The settings of
  the key follow in sections (`section` in `pi/schemas.js`): what the key is about, Controls, On
  the key, and closed sections for the rest. Use the own `dropdown()`, `checkbox()` and number
  stepper in `pi/pi.js`, never native `<select>` or checkboxes: OpenDeck's WebKitGTK draws those in
  the GTK style.
- An action that can do more than one thing lists it in `static ops` (the press choice is in the
  setting `pressKey`), and its schema uses `gestures()`. Press, hold and double press then offer
  the same choices, and the base class does the key handling.
- Settings a key cannot work without go in `static required`, so the key shows the setup image
  (gray glyph, yellow dot) until they are set. Errors an action throws show up on its settings
  page: write them as short English sentences and add the German text.
- Everything a user can read comes in English and German. Write the English text in the code and
  wrap it in `t()` (from `plugin/lib/i18n.js`), then add the German text there. Action names and
  tooltips for the OpenDeck list are in `scripts/manifest.mjs` (it writes `de.json`). Messages the
  plugin sends to a settings page stay English, the page translates them with `translateMessage()`.
  `make check` fails when a German text is missing.
- Keys look like the other FeloDeck plugins too: no background, bold white glyph in the middle,
  gray when off. Dials with their own touch strip layout use `layouts/*.json`.
- Test in the real apps. First check if a real Stream Deck is plugged in (`lsusb | grep -i 0fd9`,
  Elgato's USB vendor ID). If one is, test on it, but only in new profiles of its own, with a name
  that is not taken yet (for example "OBS Tools Test"). Never change or delete the profiles that
  are already on it, they are in use. Note which profile was selected before
  (`~/.config/opendeck/profiles/<device>.json`) and select it again when done. `make-profile.mjs`
  only writes profiles for the virtual deck.
- Without a real Stream Deck, use the virtual deck: `make install install-virtual-deck`, restart
  OpenDeck, put actions on keys with `tools/virtual-deck/make-profile.mjs` (OpenDeck must be
  stopped), then run the matching `test/e2e/run*.mjs`.
- `make check` runs the unit tests. OBS that is busy encoding (a weak VM) can make the end-to-end
  tests flaky, run them again before hunting a bug.

## Releases

1. Read `git log <last tag>..HEAD` and pick the version: only fixes → patch, something new → minor,
   something that breaks or needs the user to act → major.
2. Bump `VERSION`, run `make manifest` and add the entry at the top of `CHANGELOG.md`, in one commit
   (`chore: 1.4.0`).
3. Push, then push the tag: `git tag v1.4.0 && git push origin main v1.4.0`.
4. `make package`, then `gh release create v1.4.0` with the entry from `CHANGELOG.md` and a short
   install part as notes, and only the plugin file, named `OBS-Tools-v1.4.0.streamDeckPlugin`. Never
   attach a `.zip` as well: the plugin store of OpenDeck then asks which file to download.

A minor or major release starts with a welcome sentence and has a `### Highlights` list. A patch
release is just a sentence or two. Write for users: what they notice, not how the code does it.

## README

- The virtual deck is a test tool. The README does not mention it.
- New UI (a key, a settings page) gets a screenshot in the README, next to the
  text about it. A real screenshot of it running, in English.
- When a screen changes, take its screenshot again. The README never shows an old one.
