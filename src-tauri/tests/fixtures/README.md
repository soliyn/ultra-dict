# MDict test fixtures

Tiny MDict dictionaries for the automated Rust tests (see `docs/spec-v1.md`, "Testing"). Each dictionary has its own folder, because companion `.mdd` volumes and loose resource files are found by looking next to the `.mdx`.

## Regenerating

The files are generated, never hand-edited. The generator is `tools/make-fixtures` (a standalone Cargo crate using `mdictlib` 0.2.8's `MdxBuilder` and `MddBuilder`):

```sh
cd tools/make-fixtures
cargo run              # rewrite every fixture in this folder
cargo run -- --check   # exit non-zero if a committed fixture differs from a fresh build
```

Output is byte-for-byte deterministic. Both modes then reopen every fixture with `mdictlib` and check the contents listed below. To change a fixture, edit `tools/make-fixtures/src/main.rs` and rerun.

`.gitattributes` marks this folder `-text`, so git never rewrites line endings here (that would break `--check`).

## Contents

All dictionaries are MDict v2.0 and UTF-8; they are zlib-compressed and unencrypted unless noted. MDD resource keys use the usual MDict form with a leading backslash (`\img\dot.png`).

### `basic/` — `basic.mdx` + `basic.mdd`

`Title` is `<b>Basic</b> Test Dictionary`. The HTML is there on purpose, to test stripping it for the display name.

| Headword(s) | What it covers |
|---|---|
| `Polish`, `polish` | case variants: separate rows |
| `resume`, `résumé` | diacritic variants: same folded key |
| `bank` ×2 | two entries for one headword; file order is `<sup>1</sup>` then `<sup>2</sup>` |
| `went` → `go` | `@@@LINK=` redirect |
| `ping` ↔ `pong`, `self` → `self` | redirect loops |
| `chain1` → … → `chain6` | 5 hops: resolves under the 5-hop limit |
| `chain0` → `chain1` → … | 6 hops: exceeds the limit |
| `hello` | links `basic.css`, shows `img/dot.png`, plays `sound://hello.wav`, and links `entry://go` |

`basic.mdd` holds `\basic.css`, `\img\dot.png` (4×4 PNG) and `\hello.wav` (0.1 s, 8 kHz, 8-bit PCM).

### `multivolume/` — `multi.mdx` + `multi.mdd` + `multi.1.mdd`

One headword, `volume`, which uses `multi.css` (only in `multi.mdd`), `img/v1.png` (only in `multi.1.mdd`) and `shared.css`. `shared.css` is in **both** volumes, as `/* volume 0 */` in `multi.mdd` and `/* volume 1 */` in `multi.1.mdd`. A correct volume-order lookup returns the `volume 0` copy.

### `loose-css/` — `loose.mdx` + `loose.css`, no `.mdd`

One headword, `loose`, which links `loose.css`. That file is a plain file next to the `.mdx`.

### `untitled/` — `untitled.mdx`

One headword, `nameless`, and no `Title` header, so the display name falls back to the file name.

### `encrypted/` — `encrypted.mdx` (substitute)

This stands in for a registration-key dictionary (`Encrypted="1"`). `MdxBuilder` refuses to write encryption, and a real registration-key file can't be committed under a free licence. So the generator builds a plain dictionary (one headword, `secret`) and adds `Encrypted="1"` to its header XML, recomputing the header length and Adler-32 checksum.

The header says the keyword-section header is encrypted, so a reader must reject the file before reading any keys. `mdictlib` fails with `MissingPasscode`, the same error it gives a genuine file. The keyword header itself is **not** really encrypted. The fixture therefore tests detection and rejection only, not decryption, which v1 does not support anyway.

### `damaged/` — `damaged.mdx` (deliberately broken)

Eight headwords (`alpha` … `hotel`), each in its own **uncompressed** key block. After building, the generator changes the last letter of the last key block in place (`hotel` → `hotem`). The header and key-block index are untouched, so the file opens, reports 8 headwords and enumerates the first seven; reading the last block then fails because its keys no longer match the index summary. It tests that a failed import leaves no headwords behind.

### Not covered

- MDX v1.2 files and LZO compression: `MdxBuilder` writes only v2.0 with zlib or no compression. These are covered by the manual reference set instead.
- Key-index encryption (`Encrypted="2"`): the builder can't write it.

## Licence

Everything in this folder was written for this project: the dictionary text, the CSS, the PNG pixels (computed) and the WAV samples (computed square wave). It is released under CC0 1.0 (public domain dedication). The generator in `tools/make-fixtures` is MIT, like the rest of the repository.
