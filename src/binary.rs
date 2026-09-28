//! Conservative text recovery from values whose actual type is binary.
//!
//! Recovered strings are a derived view, not a lossless decoder for an unknown
//! file format. Callers must retain the original bytes. In particular, arbitrary
//! pairs of bytes are not assumed to be UTF-16 merely because they form valid
//! Unicode: an unmarked run needs evidence from ASCII or a coherent script.

use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

impl Encoding {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf16Le => "UTF-16LE",
            Self::Utf16Be => "UTF-16BE",
        }
    }
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The exact source range for a strictly decoded string. `len` is in bytes;
/// offsets exclude byte-order marks, NUL terminators, and surrounding whitespace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextSpan {
    pub offset: usize,
    pub len: usize,
    pub encoding: Encoding,
    pub text: String,
}

/// Hexadecimal representation that preserves every byte, including NULs.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut result = String::with_capacity(bytes.len().saturating_mul(3));
    for (index, byte) in bytes.iter().copied().enumerate() {
        if index != 0 {
            result.push(' ');
        }
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 15) as usize] as char);
    }
    result
}

/// Recover text from an explicitly hexadecimal textual value, without changing
/// the original cell. The entire input must match one supported representation:
/// byte pairs separated by whitespace/commas, optional per-byte `0x` prefixes,
/// C-style `\\xHH` escapes, PostgreSQL `\\x...`, SQL `X'...'`, or a compact hex
/// blob (with an optional `0x` prefix). Byte lists can use `[...]` or `{...}`.
///
/// At least four bytes are required. Compact unquoted blobs additionally need
/// more than eight bytes, so ordinary words such as `cafe` and 32/64-bit numeric
/// addresses are not automatically treated as text. GUIDs, MAC addresses,
/// hex-dump gutters, mixed prose, and malformed/truncated byte lists are rejected.
/// Inputs over one MiB are left to an explicit file/byte viewer.
pub fn readable_hex(text: &str) -> Option<String> {
    if text.len() > 1024 * 1024 || !text.is_ascii() {
        return None;
    }
    let text = text.trim_ascii();
    let input = text.as_bytes();
    if input.is_empty() {
        return None;
    }

    let (bytes, minimum) = if input.len() >= 3
        && matches!(input[0], b'x' | b'X')
        && input[1] == b'\''
        && input.last() == Some(&b'\'')
    {
        (parse_compact_hex(&input[2..input.len() - 1])?, 4)
    } else if has_hex_escape(input) {
        let rest = &input[2..];
        if rest.iter().all(u8::is_ascii_hexdigit) {
            // PostgreSQL bytea output has one prefix for the complete blob.
            (parse_compact_hex(rest)?, 4)
        } else {
            (parse_hex_escapes(input)?, 4)
        }
    } else {
        let compact = if has_hex_prefix(input) {
            &input[2..]
        } else {
            input
        };
        if compact.iter().all(u8::is_ascii_hexdigit) {
            (parse_compact_hex(compact)?, 9)
        } else {
            let list = match (input.first(), input.last()) {
                (Some(b'['), Some(b']')) | (Some(b'{'), Some(b'}')) => {
                    input[1..input.len() - 1].trim_ascii()
                }
                _ => input,
            };
            (parse_hex_list(list)?, 4)
        }
    };
    if bytes.len() < minimum {
        return None;
    }
    readable(&bytes)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn hex_pair(bytes: &[u8]) -> Option<u8> {
    Some((hex_digit(*bytes.first()?)? << 4) | hex_digit(*bytes.get(1)?)?)
}

fn has_hex_prefix(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == b'0' && matches!(bytes[1], b'x' | b'X')
}

fn has_hex_escape(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == b'\\' && matches!(bytes[1], b'x' | b'X')
}

fn parse_compact_hex(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(2) {
        return None;
    }
    bytes.chunks_exact(2).map(hex_pair).collect()
}

fn parse_hex_escapes(mut input: &[u8]) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(input.len() / 4);
    while !input.is_empty() {
        if !has_hex_escape(input) {
            return None;
        }
        bytes.push(hex_pair(input.get(2..4)?)?);
        input = input[4..].trim_ascii_start();
    }
    Some(bytes)
}

fn parse_hex_list(mut input: &[u8]) -> Option<Vec<u8>> {
    let prefixed = has_hex_prefix(input);
    let mut bytes = Vec::with_capacity(input.len() / if prefixed { 5 } else { 3 });
    while !input.is_empty() {
        if prefixed {
            if !has_hex_prefix(input) {
                return None;
            }
            input = &input[2..];
        }
        bytes.push(hex_pair(input.get(..2)?)?);
        input = &input[2..];
        if input.is_empty() {
            break;
        }
        if !input[0].is_ascii_whitespace() && input[0] != b',' {
            return None;
        }
        input = input.trim_ascii_start();
        if input.first() == Some(&b',') {
            input = input[1..].trim_ascii_start();
            if input.is_empty() {
                return None;
            }
        }
    }
    Some(bytes)
}

/// Recover displayable text, using newlines between separately recovered runs.
/// The raw bytes must be retained separately: this never substitutes invented
/// characters for invalid input and never claims to decompress/decrypt data.
pub fn readable(bytes: &[u8]) -> Option<String> {
    // For an entirely valid UTF-8 field preserve its whitespace, including line
    // endings. NUL-terminated strings are common in otherwise binary records.
    let utf8 = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let end = utf8.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
    if let Ok(text) = std::str::from_utf8(&utf8[..end])
        && !text.is_empty()
        && text.chars().all(is_text_char)
        && text.chars().any(is_word_char)
    {
        return Some(text.to_owned());
    }
    let recovered = spans(bytes);
    if recovered.is_empty() {
        None
    } else {
        Some(
            recovered
                .into_iter()
                .map(|s| s.text)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

/// Find non-overlapping UTF-8 and UTF-16 strings at any byte offset.
///
/// UTF-16 is checked in both byte orders and on both alignments. Invalid
/// sequences, controls, NULs, and replacement characters terminate a run.
/// Unmarked recovered runs require at least four meaningful characters, or two
/// complete emoji (UTF-16 surrogate pairs are decoded as single characters).
/// Incomplete final characters are left to the caller's raw-byte view. The
/// offsets are relative to this slice, so callers reading chunks can retain an
/// overlap or present long runs as adjoining segments without losing raw data.
pub fn spans(bytes: &[u8]) -> Vec<TextSpan> {
    let mut candidates = Vec::new();
    scan_utf8(bytes, &mut candidates);
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
        for parity in 0..2 {
            scan_utf16(bytes, parity, encoding, &mut candidates);
        }
    }
    choose_non_overlapping(candidates)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Script {
    Latin,
    Cyrillic,
    Greek,
    Hebrew,
    Arabic,
    Indic,
    Thai,
    Armenian,
    Georgian,
    EastAsian,
    Other,
}

fn script(c: char) -> Option<Script> {
    if c.is_ascii() || !c.is_alphabetic() {
        return None;
    }
    Some(match c as u32 {
        0x00c0..=0x024f => Script::Latin,
        0x0370..=0x03ff => Script::Greek,
        0x0400..=0x052f => Script::Cyrillic,
        0x0530..=0x058f => Script::Armenian,
        0x0590..=0x05ff => Script::Hebrew,
        0x0600..=0x06ff | 0x0750..=0x077f => Script::Arabic,
        0x0900..=0x0dff => Script::Indic,
        0x0e00..=0x0e7f => Script::Thai,
        0x10a0..=0x10ff => Script::Georgian,
        0x3040..=0x30ff | 0x3400..=0x9fff | 0xac00..=0xd7af => Script::EastAsian,
        _ => Script::Other,
    })
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32, 0x2600..=0x27bf | 0x1f000..=0x1faff)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || is_emoji(c)
}

fn is_text_char(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_graphic() || matches!(c, ' ' | '\t' | '\n' | '\r');
    }
    if c == '\u{fffd}' || c.is_control() {
        return false;
    }
    c.is_alphanumeric()
        || is_emoji(c)
        || matches!(c as u32,
            0x00a0..=0x00bf // Latin punctuation, spaces, currency
            | 0x0300..=0x036f // Combining accents
            | 0x2000..=0x200d // Spaces and emoji joiners
            | 0x2010..=0x2027 // Dashes, quotes, bullets
            | 0x202f..=0x203e // Spaces and punctuation
            | 0x20a0..=0x20cf // Currency symbols
            | 0x3000..=0x303f // East Asian punctuation
            | 0xfe00..=0xfe0f) // Variation selectors
}

fn utf8_char(bytes: &[u8], offset: usize) -> Option<(char, usize)> {
    let first = *bytes.get(offset)?;
    let width = match first {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let part = bytes.get(offset..offset.checked_add(width)?)?;
    let ch = std::str::from_utf8(part).ok()?.chars().next()?;
    Some((ch, width))
}

fn utf16_char(bytes: &[u8], offset: usize, encoding: Encoding) -> Option<(char, usize)> {
    let read = |at: usize| {
        let pair: [u8; 2] = bytes.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(match encoding {
            Encoding::Utf16Le => u16::from_le_bytes(pair),
            Encoding::Utf16Be => u16::from_be_bytes(pair),
            Encoding::Utf8 => unreachable!(),
        })
    };
    let first = read(offset)?;
    match first {
        0xd800..=0xdbff => {
            let second = read(offset + 2)?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return None;
            }
            let point = 0x10000 + (((first as u32) - 0xd800) << 10) + second as u32 - 0xdc00;
            Some((char::from_u32(point)?, 4))
        }
        0xdc00..=0xdfff => None,
        _ => Some((char::from_u32(first as u32)?, 2)),
    }
}

#[derive(Default)]
struct Run {
    start: usize,
    end: usize,
    text: String,
    words: usize,
    ascii_words: usize,
    unicode_words: usize,
    emoji: usize,
    whitespace: usize,
    script_boundaries: usize,
    rare_cyrillic: usize,
    previous_word_ascii: Option<bool>,
    script: Option<Script>,
    bom: bool,
}

impl Run {
    fn append(&mut self, offset: usize, width: usize, ch: char) {
        if self.text.is_empty() {
            // Leading separators are not evidence of text, and may be bytes
            // belonging to the binary structure immediately before the string.
            if ch.is_whitespace() {
                return;
            }
            self.start = offset;
        }
        self.text.push(ch);
        self.end = offset + width;
        if ch.is_alphanumeric() {
            let ascii = ch.is_ascii();
            if self
                .previous_word_ascii
                .is_some_and(|previous| previous != ascii)
            {
                self.script_boundaries += 1;
            }
            self.previous_word_ascii = Some(ascii);
        } else {
            self.previous_word_ascii = None;
        }
        if ch.is_whitespace() {
            self.whitespace += 1;
        }
        if matches!(ch as u32, 0x0460..=0x052f) {
            self.rare_cyrillic += 1;
        }
        if ch.is_ascii_alphanumeric() {
            self.words += 1;
            self.ascii_words += 1;
        } else if ch.is_alphanumeric() {
            self.words += 1;
            self.unicode_words += 1;
            self.script = script(ch).or(self.script);
        } else if is_emoji(ch) {
            self.words += 1;
            self.emoji += 1;
        }
    }

    fn finish(&mut self, encoding: Encoding, candidates: &mut Vec<Candidate>) {
        let mut run = std::mem::take(self);
        let wide_unmarked =
            matches!(run.script, Some(Script::EastAsian | Script::Other)) && !run.bom;
        let evidence = encoding == Encoding::Utf8
            || run.bom
            || run.ascii_words >= 3
            || (!wide_unmarked && run.unicode_words >= 4)
            || run.emoji >= 2;
        let minimum = if run.bom {
            1
        } else if run.emoji >= 2 {
            2
        } else {
            4
        };
        if run.words < minimum || !evidence {
            return;
        }
        // Trimming updates the source range in the original encoding, rather
        // than confusing UTF-8 String lengths with the underlying byte lengths.
        while run
            .text
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
        {
            let ch = run.text.pop().unwrap();
            run.end -= match encoding {
                Encoding::Utf8 => ch.len_utf8(),
                _ => ch.len_utf16() * 2,
            };
        }
        let confidence = if run.bom {
            128
        } else if encoding == Encoding::Utf8 {
            32
        } else {
            36
        };
        let len = run.end - run.start;
        let mut weight = (len as u64).saturating_mul(confidence);
        if encoding != Encoding::Utf8 && !run.bom {
            // Opposite endian + a one-byte shift can produce deceptively
            // similar Cyrillic. Natural separators support the real alignment;
            // artificial ASCII/Cyrillic word boundaries and stray extended
            // letters weaken the interpretation without deleting either span.
            weight = weight.saturating_add((run.whitespace as u64).saturating_mul(96));
            weight = weight.saturating_sub((run.script_boundaries as u64).saturating_mul(128));
            weight = weight.saturating_sub((run.rare_cyrillic as u64).saturating_mul(96));
        }
        candidates.push(Candidate {
            span: TextSpan {
                offset: run.start,
                len,
                encoding,
                text: run.text,
            },
            weight,
        });
    }
}

fn scan_utf8(bytes: &[u8], candidates: &mut Vec<Candidate>) {
    let mut run = Run::default();
    let mut offset = 0;
    while offset < bytes.len() {
        match utf8_char(bytes, offset) {
            Some(('\u{feff}', width)) => {
                run.finish(Encoding::Utf8, candidates);
                run.bom = true;
                offset += width;
            }
            Some((ch, width)) if is_text_char(ch) => {
                run.append(offset, width, ch);
                offset += width;
            }
            value => {
                run.finish(Encoding::Utf8, candidates);
                offset += value.map_or(1, |(_, width)| width);
            }
        }
    }
    run.finish(Encoding::Utf8, candidates);
}

fn scan_utf16(bytes: &[u8], parity: usize, encoding: Encoding, candidates: &mut Vec<Candidate>) {
    let mut run = Run::default();
    let mut offset = parity;
    while offset + 1 < bytes.len() {
        match utf16_char(bytes, offset, encoding) {
            Some(('\u{feff}', width)) => {
                run.finish(encoding, candidates);
                run.bom = true;
                offset += width;
            }
            Some((ch, width)) if is_text_char(ch) => {
                let next_script = script(ch);
                let switches_script =
                    run.script.is_some() && next_script.is_some() && run.script != next_script;
                if switches_script && !run.bom {
                    run.finish(encoding, candidates);
                }
                // CJK covers a large part of the BMP: arbitrary binary pairs
                // frequently look like its characters. Without a BOM or prior
                // text anchor they are insufficient evidence for UTF-16.
                let weak_start = matches!(next_script, Some(Script::EastAsian | Script::Other));
                if !weak_start || !run.text.is_empty() || run.bom {
                    run.append(offset, width, ch);
                }
                offset += width;
            }
            value => {
                run.finish(encoding, candidates);
                offset += value.map_or(2, |(_, width)| width);
            }
        }
    }
    run.finish(encoding, candidates);
}

struct Candidate {
    span: TextSpan,
    weight: u64,
}

fn choose_non_overlapping(mut candidates: Vec<Candidate>) -> Vec<TextSpan> {
    // Weighted interval scheduling resolves opposite-byte-order/alignment
    // interpretations without quadratic pairwise comparisons on large files.
    candidates.sort_unstable_by_key(|c| (c.span.offset + c.span.len, c.span.offset));
    let mut scores = vec![0u64; candidates.len() + 1];
    let mut predecessors = Vec::with_capacity(candidates.len());
    for (index, candidate) in candidates.iter().enumerate() {
        let predecessor = candidates[..index]
            .partition_point(|other| other.span.offset + other.span.len <= candidate.span.offset);
        predecessors.push(predecessor);
        let including = scores[predecessor].saturating_add(candidate.weight);
        scores[index + 1] = scores[index].max(including);
    }
    let mut selected = Vec::new();
    let mut index = candidates.len();
    while index > 0 {
        let candidate = &candidates[index - 1];
        let predecessor = predecessors[index - 1];
        if scores[predecessor].saturating_add(candidate.weight) > scores[index - 1] {
            selected.push(index - 1);
            index = predecessor;
        } else {
            index -= 1;
        }
    }
    selected.reverse();
    let mut selected = selected.into_iter().peekable();
    candidates
        .into_iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            if selected.peek() == Some(&index) {
                selected.next();
                Some(candidate.span)
            } else {
                None
            }
        })
        .collect()
}
