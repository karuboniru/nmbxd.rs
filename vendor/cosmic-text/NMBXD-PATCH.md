# Local emoji presentation patch

Base: cosmic-text 0.14.2, copied from the crates.io distribution. Upstream:
https://github.com/pop-os/cosmic-text/tree/v0.14.2

The original MIT and Apache licenses are retained. Only build sources, manifests,
upstream README, and the font fixtures needed for local unit tests are included;
unused upstream integration test and benchmark target declarations were removed
from Cargo.toml. Cargo.toml.orig preserves the upstream manifest.

Modified files: src/lib.rs and src/shape.rs. New file:
src/emoji_presentation.rs. The original fallback strategy only retries when a
glyph is missing. Ordinary text fonts may already contain monochrome emoji,
placeholder flag letters, or incomplete emoji sequences. Consequently an
installed color emoji font alone does not reliably render full clusters.

The patch detects emoji presentation per extended grapheme cluster and tries
Noto Color Emoji first at its regular weight. FE0E retains text presentation;
FE0F, keycaps, default emoji, skin tones, flags and ZWJ sequences use the emoji
font. Intervening ordinary text remains in complete shaping runs, preserving
ligatures and contextual scripts. The original UTF-8 byte indices and metadata
are retained. RTL segment groups are emitted in reverse logical order, matching
Rustybuzz's visual ordering within each segment. If Noto Color Emoji is absent,
the original font selection remains in effect.

The Unicode property tables were generated from Unicode 17.0:
https://www.unicode.org/Public/17.0.0/ucd/emoji/emoji-data.txt
The original data and Unicode license are retained in unicode/.

Unit tests cover presentation selectors, real color glyph rasterization for the
application's emoji fixture, preference over a simulated text font with scalar
coverage, selected text fonts, ordinary ligatures and byte offsets, and mixed
Arabic/emoji RTL run ordering. Test fonts Inter and Noto Sans Arabic come from
the upstream cosmic-text distribution; their OFL licenses are in test-fonts/.

From the application repository, run:

```sh
cargo test -p cosmic-text --lib
```

Revisit this patch when updating GPUI/cosmic-text. GPUI 0.2.2 also requires the
compatible CBDT Noto Color Emoji asset registered by src/emoji.rs; this patch
does not add COLRv1 rasterization support to its Swash renderer.

Build integration: the project workspace owns build profiles. Explicit elided
lifetimes in attrs.rs/buffer.rs and a redundant log argument borrow were cleaned
up for Rust 1.98's current lints; these do not alter shaping behavior.
