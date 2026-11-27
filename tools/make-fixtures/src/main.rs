//! Generates the tiny MDict test dictionaries in `src-tauri/tests/fixtures`.
//!
//! Usage (from `tools/make-fixtures/`):
//!
//! ```text
//! cargo run                 # (re)write the fixtures
//! cargo run -- --check      # fail if the committed fixtures differ from a fresh build
//! cargo run -- <OUT_DIR>    # write somewhere else
//! ```
//!
//! Output is byte-for-byte deterministic: no timestamps, no floating point,
//! fixed insertion order. After writing (or checking), every fixture is
//! reopened with `mdictlib` and its expected contents are asserted.
//!
//! All dictionary text, CSS, the image and the sound are written in this file
//! and are original; see `src-tauri/tests/fixtures/README.md` for the licence.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mdictlib::{Error, MddBuilder, MddFile, MdxBuilder, MdxFile, WriteCompression, WriteOptions};

/// Relative path (with `/` separators) → file bytes.
type Files = BTreeMap<&'static str, Vec<u8>>;

fn main() -> ExitCode {
    let mut check = false;
    let mut out_dir = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--check" => check = true,
            "-h" | "--help" => {
                println!("usage: make-fixtures [--check] [OUT_DIR]");
                return ExitCode::SUCCESS;
            }
            _ if out_dir.is_none() => out_dir = Some(PathBuf::from(arg)),
            _ => {
                eprintln!("unexpected argument: {arg}");
                return ExitCode::FAILURE;
            }
        }
    }
    let out_dir = out_dir.unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/tests/fixtures")
    });

    let files = build_all();

    if check {
        let mut stale = Vec::new();
        for (rel, bytes) in &files {
            match fs::read(out_dir.join(rel)) {
                Ok(on_disk) if on_disk == *bytes => {}
                Ok(_) => stale.push(format!("{rel}: differs")),
                Err(e) => stale.push(format!("{rel}: {e}")),
            }
        }
        if !stale.is_empty() {
            eprintln!("fixtures are out of date (run `cargo run` to regenerate):");
            for line in stale {
                eprintln!("  {line}");
            }
            return ExitCode::FAILURE;
        }
        println!("{} fixture files match a fresh build", files.len());
    } else {
        for (rel, bytes) in &files {
            let path = out_dir.join(rel);
            fs::create_dir_all(path.parent().unwrap()).expect("create fixture directory");
            fs::write(&path, bytes).expect("write fixture");
            println!("wrote {rel} ({} bytes)", bytes.len());
        }
        let total: usize = files.values().map(Vec::len).sum();
        println!("total {total} bytes in {} files", files.len());
    }

    verify(&out_dir);
    println!("verified with mdictlib");
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// Fixture content
// ---------------------------------------------------------------------------

fn build_all() -> Files {
    let mut files = Files::new();
    build_basic(&mut files);
    build_multivolume(&mut files);
    build_loose_css(&mut files);
    build_untitled(&mut files);
    build_encrypted(&mut files);
    build_damaged(&mut files);
    files
}

/// `basic/`: headword edge cases, redirects, and resources in a single `.mdd`.
fn build_basic(files: &mut Files) {
    let mut mdx = MdxBuilder::new();
    mdx.header_attribute("Title", "<b>Basic</b> Test Dictionary").unwrap();
    mdx.header_attribute("Description", "Fixture for ultra-dict automated tests.")
        .unwrap();
    for (key, body) in BASIC_ENTRIES {
        mdx.add_entry(key, body).unwrap();
    }
    files.insert("basic/basic.mdx", finish_mdx(mdx));

    let mut mdd = MddBuilder::new();
    mdd.header_attribute("Title", "Basic Test Dictionary").unwrap();
    mdd.add_resource("\\basic.css", BASIC_CSS.as_bytes()).unwrap();
    mdd.add_resource("\\img\\dot.png", &png_checker(4, [0x20, 0x60, 0xc0]))
        .unwrap();
    mdd.add_resource("\\hello.wav", &wav_square()).unwrap();
    files.insert("basic/basic.mdd", finish_mdd(mdd));
}

const BASIC_ENTRIES: &[(&str, &str)] = &[
    // Case variants: two distinct headwords.
    (
        "Polish",
        r#"<link rel="stylesheet" href="basic.css"><div class="entry"><b>Polish</b> <i>adj.</i> of or from Poland.</div>"#,
    ),
    (
        "polish",
        r#"<link rel="stylesheet" href="basic.css"><div class="entry"><b>polish</b> <i>v.</i> to make a surface smooth and shiny.</div>"#,
    ),
    // Diacritic variants: fold to the same key "resume".
    (
        "resume",
        r#"<div class="entry"><b>resume</b> <i>v.</i> to begin again after a pause.</div>"#,
    ),
    (
        "résumé",
        r#"<div class="entry"><b>résumé</b> <i>n.</i> a short summary of one's career.</div>"#,
    ),
    // One headword, two entries in the same dictionary (file order matters).
    (
        "bank",
        r#"<div class="entry"><b>bank</b> <sup>1</sup> <i>n.</i> the land along a river.</div>"#,
    ),
    (
        "bank",
        r#"<div class="entry"><b>bank</b> <sup>2</sup> <i>n.</i> a place that keeps money.</div>"#,
    ),
    // Redirect: went -> go.
    ("went", "@@@LINK=go\r\n"),
    (
        "go",
        r#"<div class="entry"><b>go</b> <i>v.</i> (went, gone) to move from one place to another.</div>"#,
    ),
    // Redirect loops: two-word cycle and a self-redirect.
    ("ping", "@@@LINK=pong\r\n"),
    ("pong", "@@@LINK=ping\r\n"),
    ("self", "@@@LINK=self\r\n"),
    // Redirect chains around the 5-hop limit:
    // chain1 -> ... -> chain6 is 5 hops (resolves); chain0 adds a 6th (refused).
    ("chain0", "@@@LINK=chain1\r\n"),
    ("chain1", "@@@LINK=chain2\r\n"),
    ("chain2", "@@@LINK=chain3\r\n"),
    ("chain3", "@@@LINK=chain4\r\n"),
    ("chain4", "@@@LINK=chain5\r\n"),
    ("chain5", "@@@LINK=chain6\r\n"),
    (
        "chain6",
        r#"<div class="entry"><b>chain6</b> the end of the redirect chain.</div>"#,
    ),
    // Resources from the .mdd: CSS, image, sound; plus an entry:// link.
    (
        "hello",
        concat!(
            r#"<link rel="stylesheet" type="text/css" href="basic.css">"#,
            r#"<div class="entry"><b>hello</b> "#,
            r#"<a class="snd" href="sound://hello.wav">&#9654;</a> "#,
            r#"<i>interj.</i> a greeting. "#,
            r#"<img src="img/dot.png" alt="dot"> "#,
            r#"See also <a href="entry://go">go</a>.</div>"#,
        ),
    ),
];

const BASIC_CSS: &str = "\
.entry { font-family: serif; color: #123; }
.entry b { color: #2060c0; }
.snd { text-decoration: none; }
";

/// `multivolume/`: resources split across `multi.mdd` and `multi.1.mdd`.
///
/// `shared.css` exists in both volumes with different content, so a reader
/// that searches in volume order must return the `multi.mdd` copy.
fn build_multivolume(files: &mut Files) {
    let mut mdx = MdxBuilder::new();
    mdx.header_attribute("Title", "Multi-volume Test Dictionary").unwrap();
    mdx.add_entry(
        "volume",
        concat!(
            r#"<link rel="stylesheet" href="multi.css">"#,
            r#"<link rel="stylesheet" href="shared.css">"#,
            r#"<div class="entry"><b>volume</b> <i>n.</i> one book of a set. "#,
            r#"<img src="img/v1.png" alt="from volume 1"></div>"#,
        ),
    )
    .unwrap();
    files.insert("multivolume/multi.mdx", finish_mdx(mdx));

    let mut vol0 = MddBuilder::new();
    vol0.add_resource("\\multi.css", b".entry { color: #303030; }\n")
        .unwrap();
    vol0.add_resource("\\shared.css", b"/* volume 0 */\n").unwrap();
    files.insert("multivolume/multi.mdd", finish_mdd(vol0));

    let mut vol1 = MddBuilder::new();
    vol1.add_resource("\\img\\v1.png", &png_checker(4, [0xc0, 0x40, 0x20]))
        .unwrap();
    vol1.add_resource("\\shared.css", b"/* volume 1 */\n").unwrap();
    files.insert("multivolume/multi.1.mdd", finish_mdd(vol1));
}

/// `loose-css/`: no `.mdd`; the stylesheet sits next to the `.mdx`.
fn build_loose_css(files: &mut Files) {
    let mut mdx = MdxBuilder::new();
    mdx.header_attribute("Title", "Loose CSS Test Dictionary").unwrap();
    mdx.add_entry(
        "loose",
        r#"<link rel="stylesheet" href="loose.css"><div class="entry"><b>loose</b> <i>adj.</i> not firmly fixed.</div>"#,
    )
    .unwrap();
    files.insert("loose-css/loose.mdx", finish_mdx(mdx));
    files.insert(
        "loose-css/loose.css",
        b".entry { font-family: sans-serif; color: #604020; }\n".to_vec(),
    );
}

/// `untitled/`: no `Title` header, so the display name falls back to the
/// file name.
fn build_untitled(files: &mut Files) {
    let mut mdx = MdxBuilder::new();
    mdx.add_entry("nameless", "<div><b>nameless</b> having no name.</div>")
        .unwrap();
    files.insert("untitled/untitled.mdx", finish_mdx(mdx));
}

/// `encrypted/`: substitute for a registration-key (`Encrypted="1"`) file.
///
/// `MdxBuilder` refuses to write encryption, so this builds a plaintext
/// dictionary and adds `Encrypted="1"` to its header XML (fixing the header
/// length and checksum). Readers that honour the flag ask for a passcode
/// before touching the keyword section, exactly as for a genuine file; the
/// keyword header itself is left unencrypted, so this does not exercise
/// decryption.
fn build_encrypted(files: &mut Files) {
    let mut mdx = MdxBuilder::new();
    mdx.header_attribute("Title", "Encrypted Test Dictionary").unwrap();
    mdx.add_entry("secret", "<div>needs a registration key</div>")
        .unwrap();
    let plain = finish_mdx(mdx);
    files.insert(
        "encrypted/encrypted.mdx",
        set_header_attribute(&plain, "Encrypted", "1"),
    );
}

/// Headwords of `damaged/`, in file order.
const DAMAGED_KEYS: &[&str] = &[
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
];

/// `damaged/`: opens cleanly but fails part-way through its headwords.
///
/// Every key goes in its own uncompressed key block, then the last letter of
/// the last key (`hotel` → `hotem`) is changed in place. The header and the
/// key-block index are intact, so the file opens and the first keys
/// enumerate; the last block no longer matches its index summary, so reading
/// it fails. This exercises "a failed import leaves nothing behind".
fn build_damaged(files: &mut Files) {
    let options = WriteOptions::new()
        .with_compression(WriteCompression::None)
        .with_key_block_target_bytes(1);
    let mut mdx = MdxBuilder::with_options(options);
    mdx.header_attribute("Title", "Damaged Test Dictionary").unwrap();
    for key in DAMAGED_KEYS {
        mdx.add_entry(key, &format!("<div><b>{key}</b></div>")).unwrap();
    }
    let mut file = finish_mdx(mdx);
    // Each uncompressed key row ends with the key and a NUL.
    let end = key_blocks_end(&file);
    assert_eq!(&file[end - 6..end], b"hotel ", "last key block layout");
    file[end - 2] = b'm';
    files.insert("damaged/damaged.mdx", file);
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

fn verify(dir: &Path) {
    // basic
    let mdx = MdxFile::open(dir.join("basic/basic.mdx")).expect("open basic.mdx");
    assert_eq!(mdx.header().title(), Some("<b>Basic</b> Test Dictionary"));
    let entries: Vec<(String, String)> = mdx
        .entries()
        .map(|e| {
            let e = e.unwrap();
            (e.key().to_owned(), e.text().to_owned())
        })
        .collect();
    assert_eq!(entries.len(), BASIC_ENTRIES.len());
    for (key, body) in BASIC_ENTRIES {
        assert!(
            entries.iter().any(|(k, b)| k == key && b == body),
            "basic.mdx is missing entry {key:?}"
        );
    }
    for key in ["Polish", "polish", "resume", "résumé"] {
        assert!(entries.iter().any(|(k, _)| k == key), "missing {key:?}");
    }
    let banks: Vec<&str> = entries
        .iter()
        .filter(|(k, _)| k == "bank")
        .map(|(_, b)| b.as_str())
        .collect();
    assert_eq!(banks.len(), 2, "bank has two entries");
    assert!(banks[0].contains("<sup>1</sup>") && banks[1].contains("<sup>2</sup>"));

    let mdd = MddFile::open(dir.join("basic/basic.mdd")).expect("open basic.mdd");
    assert_eq!(mdd_bytes(&mdd, "\\basic.css"), BASIC_CSS.as_bytes());
    assert!(mdd_bytes(&mdd, "\\img\\dot.png").starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(mdd_bytes(&mdd, "\\hello.wav").starts_with(b"RIFF"));

    // multivolume
    let mdx = MdxFile::open(dir.join("multivolume/multi.mdx")).expect("open multi.mdx");
    assert_eq!(mdx.len(), 1);
    let vol0 = MddFile::open(dir.join("multivolume/multi.mdd")).expect("open multi.mdd");
    let vol1 = MddFile::open(dir.join("multivolume/multi.1.mdd")).expect("open multi.1.mdd");
    assert!(vol0.lookup("\\multi.css").unwrap().is_some());
    assert!(vol0.lookup("\\img\\v1.png").unwrap().is_none());
    assert!(vol1.lookup("\\img\\v1.png").unwrap().is_some());
    assert_eq!(mdd_bytes(&vol0, "\\shared.css"), b"/* volume 0 */\n");
    assert_eq!(mdd_bytes(&vol1, "\\shared.css"), b"/* volume 1 */\n");

    // loose-css
    let mdx = MdxFile::open(dir.join("loose-css/loose.mdx")).expect("open loose.mdx");
    assert_eq!(mdx.len(), 1);
    assert!(dir.join("loose-css/loose.css").is_file());
    assert!(!dir.join("loose-css/loose.mdd").exists());

    // untitled
    let mdx = MdxFile::open(dir.join("untitled/untitled.mdx")).expect("open untitled.mdx");
    assert_eq!(mdx.header().title(), None);

    // encrypted
    let path = dir.join("encrypted/encrypted.mdx");
    match MdxFile::open(&path) {
        Err(Error::MissingPasscode) => {}
        Err(other) => panic!("encrypted.mdx: expected MissingPasscode, got {other}"),
        Ok(_) => panic!("encrypted.mdx opened without a passcode"),
    }

    // damaged
    let mdx = MdxFile::open(dir.join("damaged/damaged.mdx")).expect("open damaged.mdx");
    assert_eq!(mdx.len(), DAMAGED_KEYS.len() as u64);
    let keys: Vec<_> = mdx.keys().collect();
    let good = keys.iter().take_while(|k| k.is_ok()).count();
    assert!(
        good > 0 && good < DAMAGED_KEYS.len(),
        "damaged.mdx: {good} keys before the error, expected some but not all"
    );
    assert!(keys[good].is_err(), "damaged.mdx: enumeration must end in an error");
}

fn mdd_bytes(mdd: &MddFile, key: &str) -> Vec<u8> {
    mdd.lookup(key)
        .unwrap()
        .unwrap_or_else(|| panic!("resource {key:?} not found"))
        .bytes()
        .to_vec()
}

// ---------------------------------------------------------------------------
// MDict helpers
// ---------------------------------------------------------------------------

fn finish_mdx(builder: MdxBuilder) -> Vec<u8> {
    let mut out = Vec::new();
    builder.finish(&mut out).expect("write mdx");
    out
}

fn finish_mdd(builder: MddBuilder) -> Vec<u8> {
    let mut out = Vec::new();
    builder.finish(&mut out).expect("write mdd");
    out
}

/// Offset just past the key blocks of an unencrypted MDict v2 file.
///
/// After the header comes the keyword-section header (five `u64 BE`: block
/// count, entry count, key-index decompressed length, key-index compressed
/// length, key-blocks length) and its `u32` Adler-32, then the key index,
/// then the key blocks.
fn key_blocks_end(file: &[u8]) -> usize {
    let header_len = u32::from_be_bytes(file[0..4].try_into().unwrap()) as usize;
    let section = 4 + header_len + 4;
    let field = |i: usize| {
        let at = section + 8 * i;
        u64::from_be_bytes(file[at..at + 8].try_into().unwrap()) as usize
    };
    section + 5 * 8 + 4 + field(3) + field(4)
}

/// Inserts `name="value"` into the header XML of an MDict v2 file.
///
/// Layout: `u32 BE xml_len | xml (UTF-16LE) | u32 LE adler32(xml) | rest`.
/// Everything after the header is located relative to its end, so changing
/// the header length is safe.
fn set_header_attribute(file: &[u8], name: &str, value: &str) -> Vec<u8> {
    let len = u32::from_be_bytes(file[0..4].try_into().unwrap()) as usize;
    let xml_bytes = &file[4..4 + len];
    let rest = &file[4 + len + 4..];

    let units: Vec<u16> = xml_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .collect();
    let xml = String::from_utf16(&units).expect("header is UTF-16LE");
    assert!(!xml.contains(&format!(" {name}=")), "{name} already set");
    let tail = xml.rfind("/>").expect("self-closing header element");
    let patched = format!("{} {name}=\"{value}\"{}", &xml[..tail], &xml[tail..]);
    let patched: Vec<u8> = patched.encode_utf16().flat_map(u16::to_le_bytes).collect();

    let mut out = Vec::with_capacity(file.len() + 32);
    out.extend_from_slice(&(patched.len() as u32).to_be_bytes());
    out.extend_from_slice(&patched);
    out.extend_from_slice(&adler32(&patched).to_le_bytes());
    out.extend_from_slice(rest);
    out
}

// ---------------------------------------------------------------------------
// Tiny media generators (integer-only, so output is identical on every platform)
// ---------------------------------------------------------------------------

/// A `size`×`size` RGB PNG: a checkerboard of `color` and white.
fn png_checker(size: u32, color: [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..size {
        raw.push(0); // filter: none
        for x in 0..size {
            if (x + y) % 2 == 0 {
                raw.extend_from_slice(&color);
            } else {
                raw.extend_from_slice(&[0xff, 0xff, 0xff]);
            }
        }
    }
    // zlib stream with one stored (uncompressed) deflate block.
    let mut zlib = vec![0x78, 0x01, 0x01];
    let n = raw.len() as u16;
    zlib.extend_from_slice(&n.to_le_bytes());
    zlib.extend_from_slice(&(!n).to_le_bytes());
    zlib.extend_from_slice(&raw);
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, truecolour, deflate, no filter, no interlace

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png_chunk(&mut png, b"IHDR", &ihdr);
    png_chunk(&mut png, b"IDAT", &zlib);
    png_chunk(&mut png, b"IEND", &[]);
    png
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = kind.to_vec();
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// 0.1 s of a 400 Hz square wave: 8 kHz, 8-bit unsigned PCM, mono.
fn wav_square() -> Vec<u8> {
    const RATE: u32 = 8000;
    let samples: Vec<u8> = (0..RATE / 10)
        .map(|i| if (i / 10) % 2 == 0 { 0xa0 } else { 0x60 })
        .collect();
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes()); // byte rate
    wav.extend_from_slice(&1u16.to_le_bytes()); // block align
    wav.extend_from_slice(&8u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    wav.extend_from_slice(&samples);
    wav
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in bytes {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}
