use timeline::binary::{self, Encoding};

fn utf16(text: &str, little_endian: bool) -> Vec<u8> {
    text.encode_utf16()
        .flat_map(|unit| {
            if little_endian {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            }
        })
        .collect()
}

#[test]
fn recovers_synthetic_binary_url_with_exact_bytes() {
    let bytes: Vec<u8> = include_str!("fixtures/binary-url.hex")
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect();
    let expected = "http://example.test/filestreamingservice/files/00000000-0000-4000-8000-000000000000?P1=1234567890&P2=404&P3=2&P4=SyntheticFixture0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789%2bExamplePayload%3d%3d";
    let spans = binary::spans(&bytes);
    assert_eq!(spans.len(), 1, "{spans:?}");
    assert_eq!(spans[0].offset, 16);
    assert_eq!(spans[0].len, expected.len() * 2);
    assert_eq!(spans[0].encoding, Encoding::Utf16Le);
    assert_eq!(spans[0].text, expected);
    assert_eq!(binary::readable(&bytes).as_deref(), Some(expected));
    assert_eq!(
        binary::hex(&bytes),
        include_str!("fixtures/binary-url.hex").trim()
    );
}

#[test]
fn utf16_at_both_alignments_and_both_byte_orders_retains_cyrillic_and_surrogates() {
    for little in [true, false] {
        for offset in [0, 1, 6, 7] {
            let expected = "C:\\Пользователи\\Анна\\отчёт 😀.dat";
            let payload = utf16(expected, little);
            let mut bytes = vec![0xff; offset];
            bytes.extend(&payload);
            bytes.extend([0, 0, 0xfe, 0xff, 0x0f]);
            let spans = binary::spans(&bytes);
            assert_eq!(
                spans.len(),
                1,
                "little={little}, offset={offset}: {spans:?}"
            );
            assert_eq!(spans[0].offset, offset);
            assert_eq!(spans[0].len, payload.len());
            assert_eq!(spans[0].text, expected);
            assert_eq!(
                spans[0].encoding,
                if little {
                    Encoding::Utf16Le
                } else {
                    Encoding::Utf16Be
                }
            );
        }
    }
}

#[test]
fn unmarked_cyrillic_without_any_ascii_is_recovered() {
    for little in [true, false] {
        let expected = "Проверка восстановления кириллицы";
        let mut bytes = vec![0xff];
        bytes.extend(utf16(expected, little));
        bytes.extend([0, 0]);
        assert_eq!(binary::readable(&bytes).as_deref(), Some(expected));
    }
}

#[test]
fn separate_strings_keep_offsets_encodings_and_repeated_values() {
    let mut bytes = vec![0xff, 0x01];
    let first = bytes.len();
    bytes.extend(b"ASCII message");
    bytes.extend([0, 0xff]);
    let second = bytes.len();
    bytes.extend(utf16("Привет, мир! 😀", true));
    bytes.extend([0, 0, 0xff]);
    let third = bytes.len();
    bytes.extend("UTF-8: русский 🌍".as_bytes());
    bytes.extend([0, 0xff]);
    let fourth = bytes.len();
    bytes.extend(b"ASCII message");
    let spans = binary::spans(&bytes);
    assert_eq!(
        spans.iter().map(|span| span.offset).collect::<Vec<_>>(),
        [first, second, third, fourth]
    );
    assert_eq!(
        spans.iter().map(|span| span.encoding).collect::<Vec<_>>(),
        [
            Encoding::Utf8,
            Encoding::Utf16Le,
            Encoding::Utf8,
            Encoding::Utf8
        ]
    );
    assert_eq!(
        binary::readable(&bytes).as_deref(),
        Some("ASCII message\nПривет, мир! 😀\nUTF-8: русский 🌍\nASCII message")
    );
}

#[test]
fn strict_decoding_never_inserts_replacement_characters() {
    let mut bytes = b"valid before".to_vec();
    bytes.extend([0xf0, 0x9f]); // truncated UTF-8
    bytes.extend([0, 0xff]);
    bytes.extend(utf16("valid wide", true));
    bytes.extend([0x3d, 0xd8]); // unpaired high surrogate
    bytes.extend([0, 0]);
    bytes.extend(utf16("another text", true));
    bytes.push(0x7f); // incomplete final UTF-16 code unit
    let spans = binary::spans(&bytes);
    assert!(spans.iter().any(|s| s.text == "valid before"));
    assert!(spans.iter().any(|s| s.text == "valid wide"));
    assert!(spans.iter().any(|s| s.text == "another text"));
    for span in spans {
        assert!(!span.text.contains('\u{fffd}'));
        assert!(span.offset + span.len <= bytes.len());
        let original = &bytes[span.offset..span.offset + span.len];
        let recovered = match span.encoding {
            Encoding::Utf8 => span.text.as_bytes().to_vec(),
            Encoding::Utf16Le => utf16(&span.text, true),
            Encoding::Utf16Be => utf16(&span.text, false),
        };
        assert_eq!(original, recovered);
    }
    assert_eq!(binary::readable(&[0xff, 0xc0, 0x80, 0, 0xd8, 0, 0]), None);
}

#[test]
fn random_prefix_numeric_bytes_and_false_wide_characters_remain_binary() {
    let fixture: Vec<u8> = include_str!("fixtures/binary-url.hex")
        .split_whitespace()
        .take(16)
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect();
    assert_eq!(binary::readable(&fixture), None);
    assert!(binary::spans(&[0xff; 1024]).is_empty());
    assert!(binary::spans(&[0; 1024]).is_empty());
    // Valid Unicode by itself is not proof that unknown binary is UTF-16.
    let wide_noise: Vec<u8> = [0x9167u16, 0x749b, 0x987d, 0x5ff7, 0x8e93, 0x5a2b]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert!(binary::spans(&wide_noise).is_empty());
    assert_eq!(binary::readable(&[1, 2, 3, 4, 5, 6, 7, 8]), None);
}

#[test]
fn bom_allows_short_or_east_asian_text_and_is_not_part_of_the_span() {
    for little in [true, false] {
        for expected in ["é", "測試文件", "短い文章です"] {
            let mut bytes = vec![0xee];
            bytes.extend(if little { [0xff, 0xfe] } else { [0xfe, 0xff] });
            bytes.extend(utf16(expected, little));
            bytes.extend([0, 0]);
            let spans = binary::spans(&bytes);
            assert_eq!(spans.len(), 1, "{spans:?}");
            assert_eq!(
                spans[0].offset, 3,
                "little={little} expected={expected}: {spans:?}"
            );
            assert_eq!(spans[0].text, expected);
        }
    }
}

#[test]
fn complete_utf8_fields_preserve_multiline_text_and_whitespace() {
    let text = "  Первая строка\r\n\tВторая строка 😀\r\n";
    assert_eq!(binary::readable(text.as_bytes()).as_deref(), Some(text));
    let mut terminated = text.as_bytes().to_vec();
    terminated.extend([0, 0]);
    assert_eq!(binary::readable(&terminated).as_deref(), Some(text));
    assert_eq!(binary::hex(&[]), "");
    assert_eq!(binary::hex(&[0, 1, 0xab, 0xff]), "00 01 AB FF");
}

#[test]
fn long_strings_are_not_capped_or_truncated() {
    let text = "long text Привет 😀 / path ".repeat(50_000);
    let mut bytes = vec![0xff];
    bytes.extend(utf16(&text, true));
    bytes.extend([0, 0]);
    let spans = binary::spans(&bytes);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].offset, 1);
    assert_eq!(spans[0].text, text.trim_end());
    assert_eq!(spans[0].len, utf16(text.trim_end(), true).len());
}

#[test]
fn paired_emoji_surrogates_survive_without_neighboring_ascii() {
    for little in [true, false] {
        let mut bytes = vec![0xff];
        bytes.extend(utf16("😀🎉", little));
        bytes.extend([0, 0]);
        assert_eq!(binary::readable(&bytes).as_deref(), Some("😀🎉"));
    }
}

#[test]
fn every_span_round_trips_to_its_original_source_range_in_adversarial_bytes() {
    let mut state = 0x7354_efaa_184d_9b1eu64;
    for length in [1, 3, 17, 255, 4096, 16_384] {
        let mut bytes: Vec<u8> = (0..length)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        bytes.extend([0, 0xff, 0, 0]);
        bytes.extend(utf16("восстановленный текст 😀", length % 2 == 1));
        bytes.extend([0, 0]);
        let mut previous_end = 0;
        for span in binary::spans(&bytes) {
            assert!(span.offset >= previous_end, "overlapping spans");
            previous_end = span.offset + span.len;
            assert!(previous_end <= bytes.len());
            let encoded = match span.encoding {
                Encoding::Utf8 => span.text.as_bytes().to_vec(),
                Encoding::Utf16Le => utf16(&span.text, true),
                Encoding::Utf16Be => utf16(&span.text, false),
            };
            assert_eq!(encoded, bytes[span.offset..previous_end]);
            assert!(!span.text.contains('\u{fffd}'));
        }
    }
}

#[test]
fn common_hex_text_representations_recover_the_same_binary_payload() {
    let fixture_bytes: Vec<u8> = include_str!("fixtures/binary-url.hex")
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect();
    for bytes in [
        fixture_bytes,
        "Hello, русский текст 😀".as_bytes().to_vec(),
        utf16("Hello, русский текст 😀", true),
        utf16("Hello, русский текст 😀", false),
    ] {
        let expected = binary::readable(&bytes).unwrap();
        let pairs: Vec<_> = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
        let compact = pairs.join("");
        let prefixed: Vec<_> = pairs.iter().map(|pair| format!("0x{pair}")).collect();
        let escaped: Vec<_> = pairs.iter().map(|pair| format!("\\x{pair}")).collect();
        let variants = [
            pairs.join(" "),
            pairs.join("\r\n"),
            pairs.join(", "),
            compact.clone(),
            format!("0x{compact}"),
            format!("0X{}", compact.to_lowercase()),
            prefixed.join(" "),
            format!("[{}]", prefixed.join(", ")),
            format!("{{{}}}", pairs.join(",")),
            escaped.join(""),
            escaped.join("\n"),
            format!("\\x{compact}"),
            format!("X'{compact}'"),
            format!("x'{}'", compact.to_lowercase()),
        ];
        for text in variants {
            assert_eq!(
                binary::readable_hex(&text).as_deref(),
                Some(expected.as_str()),
                "{text}"
            );
            assert_eq!(
                binary::readable_hex(&format!("\t{text}\r\n")).as_deref(),
                Some(expected.as_str())
            );
        }
    }
}

#[test]
fn short_hex_needs_an_explicit_byte_representation() {
    for text in [
        "41 42 43 44",
        "41,42,43,44",
        "0x41 0x42 0x43 0x44",
        "\\x41\\x42\\x43\\x44",
        "\\x41424344",
        "X'41424344'",
        "[0x41, 0x42, 0x43, 0x44]",
    ] {
        assert_eq!(
            binary::readable_hex(text).as_deref(),
            Some("ABCD"),
            "{text}"
        );
    }
    for text in [
        "cafe",
        "deadbeef",
        "41414141",
        "0x41414141",
        "0x4142434445464748",
        "4142434445464748",
        "48:65:6C:6C:6F:21",
        "48-65-6C-6C-6F-21",
        "41424344-4546-4748-494A-4B4C4D4E4F50",
        "41 42 43",
        "X'414243'",
    ] {
        assert_eq!(binary::readable_hex(text), None, "{text}");
    }
}

#[test]
fn malformed_hex_or_mixed_prose_is_not_partially_decoded() {
    for text in [
        "",
        "  ",
        "0x",
        "\\x",
        "X''",
        "[]",
        "{}",
        "4142434445464748494",
        "0x4142434445464748494",
        "41 42 43 44 5",
        "41 42 43 44 5Z",
        "41 42 43 44 trailing text",
        "bytes: 41 42 43 44",
        "41,,42,43,44",
        "41 42 43 44,",
        "[41,42,43,44,]",
        "[41 42 43 44}",
        "0x41 42 0x43 0x44",
        "41 0x42 43 44",
        "\\x41\\x42\\x43\\x4",
        "\\x41\\x42\\x43\\x44!",
        "\\x41 42 43 44",
        "X'41424344Z'",
        "X'41424344' extra",
        "41 42 43 44\u{00a0}",
        "00000000: 41 42 43 44 |ABCD|",
    ] {
        assert_eq!(binary::readable_hex(text), None, "{text}");
    }
    assert_eq!(binary::readable_hex(&"41 ".repeat(400_000)), None);
}

#[test]
fn hex_with_no_readable_content_stays_binary() {
    for bytes in [vec![0; 32], vec![0xff; 32], vec![1, 2, 3, 4, 5, 6, 7, 8]] {
        assert_eq!(binary::readable_hex(&binary::hex(&bytes)), None);
    }
}
