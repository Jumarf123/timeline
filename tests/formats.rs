use std::{fs, sync::Arc};
use tempfile::TempDir;
use timeline::{
    data::{Dataset, OpenOptionsConfig, Progress},
    query::{self, Query},
    search::{SearchMode, TableSearch},
};
fn load(name: &str, bytes: &[u8], options: OpenOptionsConfig) -> (TempDir, Arc<Dataset>) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, bytes).unwrap();
    let data = Dataset::open(&path, &options, &Arc::default()).unwrap();
    (dir, data)
}
#[test]
fn txt_table_preserves_description_and_can_be_disabled() {
    let text=b"   #    PID Process        Type            Address          Description\n-----------------------------------------------------------------------\n0000   1256 svchost.exe    HIGH_ENTROPY    00007ffddc2b0000 Entropy:[7.30] Image ---wxc\n0001   2060 svchost.exe    HIGH_ENTROPY    00007ffe4c530000 Entropy:[7.02] Image ---wxc\n";
    let (_, data) = load("scan.txt", text, Default::default());
    assert_eq!(
        data.headers,
        ["#", "PID", "Process", "Type", "Address", "Description"]
    );
    assert_eq!(data.rows, 2);
    assert_eq!(&data.record(1).unwrap()[5], b"Entropy:[7.02] Image ---wxc");
    let (_, raw) = load(
        "scan.txt",
        text,
        OpenOptionsConfig {
            auto_text: false,
            ..Default::default()
        },
    );
    assert_eq!(raw.headers.len(), 1);
    assert_eq!(raw.rows, 4);
}
#[test]
fn source_code_is_not_mistaken_for_csv() {
    for name in ["example.js", "example.md", "example.ps1"] {
        let (_, data) = load(
            name,
            b"const a = [1, 2, 3];\n  const b = [4, 5, 6];\n",
            Default::default(),
        );
        assert_eq!(data.headers.len(), 1);
        assert_eq!(&data.record(1).unwrap()[0], b"  const b = [4, 5, 6];");
    }
}
#[test]
fn xml_keeps_attributes_repeated_elements_empty_elements_and_entities() {
    let (_,data)=load("a.xml",br#"<?xml version="1.0"?><root><empty/><entry id="1">A &amp; B</entry><entry id="2"><![CDATA[<literal>]]></entry></root>"#,Default::default());
    let records: Vec<_> = (0..data.rows).map(|i| data.record(i).unwrap()).collect();
    assert!(
        records
            .iter()
            .any(|r| &r[0] == b"/root[1]/entry[1]/@id" && &r[1] == b"1")
    );
    assert!(
        records
            .iter()
            .any(|r| &r[0] == b"/root[1]/entry[1]" && &r[1] == b"A & B")
    );
    assert!(
        records
            .iter()
            .any(|r| &r[0] == b"/root[1]/entry[2]" && &r[1] == b"<literal>")
    );
}

#[test]
fn xml_preserves_comments_whitespace_instructions_and_literal_dtd_entities() {
    let (_, data) = load(
        "report.xml",
        br#"<?xml version="1.0"?><!DOCTYPE root [<!ENTITY sample SYSTEM "file:///must-not-be-opened">]><?review keep?><root marker="  value&#10; "><!--evidence--><blank xml:space="preserve">  </blank><value>&sample;</value><![CDATA[  ]]></root>"#,
        Default::default(),
    );
    let records: Vec<_> = (0..data.rows).map(|i| data.record(i).unwrap()).collect();
    for (kind, text) in [
        ("declaration", "xml version=\"1.0\""),
        ("processing instruction", "review keep"),
        ("comment", "evidence"),
        ("whitespace", "  "),
        ("text", "&sample;"),
        ("attribute", "  value\n "),
    ] {
        assert!(
            records
                .iter()
                .any(|r| &r[2] == kind.as_bytes() && &r[1] == text.as_bytes()),
            "{kind}: {text:?}"
        );
    }
    assert!(
        data.warnings
            .iter()
            .any(|s| s.contains("entity references remain literal"))
    );
}

#[test]
fn malformed_structured_data_reports_an_error() {
    let dir = TempDir::new().unwrap();
    for (name, bytes) in [
        ("invalid.xml", b"<root><child></root>".as_slice()),
        ("invalid.evtx", b"ElfFile\0".as_slice()),
    ] {
        let path = dir.path().join(name);
        fs::write(&path, bytes).unwrap();
        assert!(
            Dataset::open(&path, &Default::default(), &Arc::default()).is_err(),
            "{name}"
        );
    }
}

#[test]
fn damaged_registry_dat_exposes_readable_fallback_and_the_original_error() {
    let (_, data) = load("damaged.dat", b"regf invalid hive", Default::default());
    assert_eq!(data.kind, "binary");
    assert!(
        data.warnings
            .iter()
            .any(|s| s.contains("Could not fully parse registry"))
    );
    assert!(
        data.record(0)
            .unwrap()
            .iter()
            .any(|v| v == b"regf invalid hive")
    );
}

fn evtx_header() -> Vec<u8> {
    let mut header = vec![0; 4096];
    header[..8].copy_from_slice(b"ElfFile\0");
    header[32..36].copy_from_slice(&128u32.to_le_bytes());
    header[36..38].copy_from_slice(&1u16.to_le_bytes());
    header[38..40].copy_from_slice(&3u16.to_le_bytes());
    header[40..42].copy_from_slice(&4096u16.to_le_bytes());
    header
}

#[test]
fn evtx_keeps_damaged_and_incomplete_chunks_visible_in_source_order() {
    let mut bytes = evtx_header();
    bytes.extend(vec![0; 65536]);
    bytes.extend(vec![0x5a; 65536]);
    bytes.extend(b"truncated tail");
    let (_, data) = load("damaged.evtx", &bytes, Default::default());
    assert_eq!(data.rows, 2);
    let error_column = data
        .headers
        .iter()
        .position(|s| s == "Parse error")
        .unwrap();
    let first = data.record(0).unwrap();
    let second = data.record(1).unwrap();
    assert!(String::from_utf8_lossy(&first[error_column]).contains("69632"));
    assert!(String::from_utf8_lossy(&second[error_column]).contains("135168"));
    assert!(String::from_utf8_lossy(&second[error_column]).contains("Incomplete"));
    assert_eq!(data.warnings.len(), 1);
}

#[test]
fn evtx_cancellation_stops_a_running_import_and_keeps_progress_observable() {
    use std::{
        io::Write,
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cancel.evtx");
    let mut file = fs::File::create(&path).unwrap();
    file.write_all(&evtx_header()).unwrap();
    // A sparse input exercises cancellation without a large fixture allocation.
    file.set_len(4096 + 65536 * 16_384).unwrap();
    drop(file);
    let progress = Arc::new(Progress::default());
    let worker_progress = Arc::clone(&progress);
    let worker = std::thread::spawn(move || timeline::formats::evtx(&path, &worker_progress));
    let started = Instant::now();
    while progress.done.load(Ordering::Relaxed) <= 4096 && !worker.is_finished() {
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(1));
    }
    let snapshot = progress.snapshot();
    assert_eq!(snapshot["phase"], "parse");
    assert_eq!(snapshot["unit"], "bytes");
    assert!(snapshot["done"].as_u64().unwrap() > 4096);
    let cancelled_at = Instant::now();
    progress.cancel();
    assert!(worker.join().unwrap().is_err());
    assert!(cancelled_at.elapsed() < Duration::from_secs(5));
}
#[test]
fn unknown_json_and_binary_are_detected_by_content() {
    let (_, json) = load(
        "capture.dat",
        br#"[{"name":"one","nested":{"x":2}},{"name":"two","new":true}]"#,
        Default::default(),
    );
    assert_eq!(json.kind, "json");
    assert_eq!(json.rows, 2);
    assert!(json.headers.contains(&"/nested/x".into()));
    let bytes: Vec<_> = (0..=255).collect();
    let (_, binary) = load("capture.unknown", &bytes, Default::default());
    assert_eq!(binary.kind, "binary");
    assert!(binary.headers.contains(&"Encoding".into()));
    let (_, raw) = load(
        "capture.unknown",
        &bytes,
        OpenOptionsConfig {
            format: timeline::data::Format::Hex,
            ..Default::default()
        },
    );
    assert_eq!(raw.kind, "hex");
    assert_eq!(raw.rows, 8);
    assert_eq!(&raw.record(7).unwrap()[0], b"00000000000000E0");
    let recovered: Vec<u8> = (0..raw.rows)
        .flat_map(|i| {
            String::from_utf8(raw.record(i).unwrap()[1].to_vec())
                .unwrap()
                .split_whitespace()
                .map(|byte| u8::from_str_radix(byte, 16).unwrap())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(recovered, bytes);
}

fn mixed_binary_url() -> (Vec<u8>, String) {
    let url = "http://example.test/filestreamingservice/files/00000000-0000-4000-8000-000000000000?P1=1234567890&P2=404&P3=2&P4=SyntheticFixture0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789%2bExamplePayload%3d%3d".to_owned();
    let mut bytes = vec![
        0x27, 0xb0, 0x24, 0x8b, 0xee, 0x1d, 0xbb, 0xba, 0x9a, 0x95, 0x35, 0x17, 0xdf, 0xb9, 0xc5,
        0x52,
    ];
    bytes.extend(url.encode_utf16().flat_map(u16::to_le_bytes));
    bytes.extend([0, 0, b'd', 0]);
    (bytes, url)
}

#[test]
fn dat_extracts_utf16_url_with_exact_byte_position() {
    let (bytes, url) = mixed_binary_url();
    let (_, data) = load("download.dat", &bytes, Default::default());
    assert_eq!(data.kind, "binary");
    let row = (0..data.rows)
        .map(|i| data.record(i).unwrap())
        .find(|r| &r[3] == url.as_bytes())
        .expect("the complete URL");
    assert_eq!(&row[0], b"0000000000000010");
    assert_eq!(
        &row[1],
        (url.encode_utf16().count() * 2).to_string().as_bytes()
    );
    assert_eq!(&row[2], b"UTF-16LE");
    assert!(
        !row.iter()
            .any(|v| String::from_utf8_lossy(v).contains('\u{fffd}'))
    );
}

#[test]
fn binary_readable_preserves_text_across_read_windows_and_handles_opaque_data() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("boundary.bin");
    let phrase = "Text across a read boundary / Пример текста 🚀";
    for prefix in [1024 * 1024 - 17, 1024 * 1024 - 2] {
        let mut bytes = vec![0; prefix];
        bytes.extend(phrase.encode_utf16().flat_map(u16::to_le_bytes));
        bytes.extend(vec![0; 100_000]);
        fs::write(&path, bytes).unwrap();
        let normalized = timeline::formats::binary_readable(&path, &Progress::default()).unwrap();
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .from_reader(normalized.file.reopen().unwrap());
        let rows: Vec<_> = reader.records().map(Result::unwrap).collect();
        let row = rows
            .iter()
            .find(|r| &r[3] == phrase)
            .expect("whole crossing string");
        assert_eq!(&row[0], format!("{prefix:016X}"));
        assert_eq!(&row[2], "UTF-16LE");
    }
    fs::write(&path, [0u8; 4096]).unwrap();
    let normalized = timeline::formats::binary_readable(&path, &Progress::default()).unwrap();
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(normalized.file.reopen().unwrap());
    let rows: Vec<_> = reader.records().map(Result::unwrap).collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(&rows[0][2], "Binary");
    assert!(rows[0][3].contains("No confidently readable text"));
}
#[test]
fn regex_examples_everything_lists_and_case_flags() {
    let query = TableSearch::new(
        r"!!.*!2025/\d{2}/\d{2}:\d{2}:\d{2}:\d{2}!0!",
        SearchMode::Regex,
        false,
    )
    .unwrap();
    assert!(query.matches_value("!!hello!2025/04/16:11:03:38!0!"));
    let query = TableSearch::new(
        r"(?i)(2023\/03\/09:17:11:30|2025\/04\/16:11:03:38)",
        SearchMode::Auto,
        true,
    )
    .unwrap();
    assert!(query.matches_value("2025/04/16:11:03:38"));
    let query = TableSearch::new(
        r#"(?i)([a-z]:\\[^:<>"]+\.\w{2,4}):([^:\s\\\]]+)"#,
        SearchMode::Auto,
        true,
    )
    .unwrap();
    assert!(query.matches_value(r"C:\Evidence\Example.EXE:Zone.Identifier"));
    let query = TableSearch::new(
        r"ext:.exe;.jar;.zip regex:(?i)(meteor|liquid[-\_]?bounce)",
        SearchMode::Auto,
        true,
    )
    .unwrap();
    assert!(query.matches_value(r"C:\tools\METEOR.JAR"));
    assert!(query.matches_value(r"C:\tools\liquid_bounce.exe:payload"));
    assert!(!query.matches_value("meteor.txt"));
    assert!(!query.matches_value(r"C:\meteor.exe\normal.txt"));
    assert!(!query.matches_value("normal.zip"));
    assert!(TableSearch::new("regex:[", SearchMode::Auto, false).is_err());
    assert!(
        TableSearch::new("regex:[", SearchMode::Text, false)
            .unwrap()
            .matches_value("regex:[")
    );
    assert!(
        TableSearch::new("(?-i)ABC", SearchMode::Auto, false)
            .unwrap()
            .matches_value("ABC")
    );
    assert!(
        !TableSearch::new("(?-i)ABC", SearchMode::Auto, false)
            .unwrap()
            .matches_value("abc")
    );
}
#[test]
fn everything_filters_do_not_mix_unrelated_cells_and_cover_late_rows() {
    let mut text = "Path,Note\nnormal.exe,meteor.txt\n".to_owned();
    for _ in 0..20000 {
        text.push_str("normal.txt,ok\n");
    }
    text.push_str("METEOR.JAR,last\n");
    let (_dir, data) = load("paths.csv", text.as_bytes(), Default::default());
    let view = query::execute(
        &data,
        &Query {
            text: "ext:exe;jar regex:(?i)meteor".into(),
            ..Default::default()
        },
        &query::infer_types(&data).unwrap(),
        &Progress::default(),
    )
    .unwrap();
    assert_eq!(view.rows, 1);
    assert_eq!(view.records(&data, 0, 1).unwrap()[0].0, 20001);
}

#[test]
fn full_user_alternation_compiles_once_and_matches_expected_files() {
    let pattern = include_str!("fixtures/everything-query.txt");
    let query = TableSearch::new(pattern, SearchMode::Auto, false).unwrap();
    for path in [
        r"C:\files\Meteor.JAR",
        r"C:\files\LIQUID_bounce.EXE",
        r"C:\files\Spawner Locator.py",
        r"C:\files\NVIDIA-control-panel.bat",
    ] {
        assert!(query.matches_value(path), "{path}");
    }
    for path in ["meteor.txt", "normal.exe", r"C:\meteor.exe\unrelated.txt"] {
        assert!(!query.matches_value(path), "{path}");
    }
}
#[test]
fn sqlite_preserves_numbers_blobs_and_quoted_table_names() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("evidence.dat");
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE \"a\"\"b\" (path TEXT, id INTEGER, data BLOB); INSERT INTO \"a\"\"b\" VALUES ('C:\\test',9007199254740993,x'00ff');").unwrap();
    }
    let data = Dataset::open(&path, &Default::default(), &Arc::default()).unwrap();
    assert_eq!(data.kind, "sqlite");
    let record = data.record(0).unwrap();
    assert!(record.iter().any(|v| v == b"9007199254740993"));
    assert!(record.iter().any(|v| v == b"00 FF"));
}

#[test]
fn sqlite_decodes_binary_text_and_preserves_unambiguous_raw_columns() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("downloads.dat");
    let (bytes, url) = mixed_binary_url();
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE downloads (payload BLOB, \"$raw/payload\" TEXT, invalid TEXT);",
        )
        .unwrap();
        db.execute(
            "INSERT INTO downloads VALUES (?1, 'original field', CAST(x'ff0058' AS TEXT))",
            [&bytes],
        )
        .unwrap();
    }
    let data = Dataset::open(&path, &Default::default(), &Arc::default()).unwrap();
    let row = data.record(0).unwrap();
    let at = |name: &str| data.headers.iter().position(|h| h == name).unwrap();
    assert_eq!(&row[at("/payload")], url.as_bytes());
    let expected_hex = bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(&row[at("$raw/payload")], expected_hex.as_bytes());
    assert_eq!(&row[at("/$raw~1payload")], b"original field");
    assert_eq!(&row[at("$raw/invalid")], b"FF 00 58");
    assert!(
        !row.iter()
            .any(|v| String::from_utf8_lossy(v).contains('\u{fffd}'))
    );
}

#[test]
fn xml_mixed_known_and_dtd_entities_keep_their_individual_meaning() {
    let (_, data) = load("entities.xml", br#"<!DOCTYPE root [<!ENTITY own "literal">]><root value="A &amp; &own; &#65;">&lt;&own;&#65;</root>"#, Default::default());
    let records: Vec<_> = (0..data.rows).map(|i| data.record(i).unwrap()).collect();
    assert!(
        records
            .iter()
            .any(|r| &r[2] == b"attribute" && &r[1] == b"A & &own; A")
    );
    assert!(
        records
            .iter()
            .any(|r| &r[2] == b"text" && &r[1] == b"<&own;A")
    );
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bad.xml");
    fs::write(&path, br#"<!DOCTYPE root><root value="&#invalid;"/>"#).unwrap();
    assert!(Dataset::open(&path, &Default::default(), &Arc::default()).is_err());
}
#[test]
fn usn_v2_records_decode_filename_reason_and_time() {
    let name: Vec<u8> = "пример.exe"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let size = (60 + name.len()).div_ceil(8) * 8;
    let mut bytes = vec![0; size];
    bytes[..4].copy_from_slice(&(size as u32).to_le_bytes());
    bytes[4..6].copy_from_slice(&2u16.to_le_bytes());
    bytes[8..16].copy_from_slice(&42u64.to_le_bytes());
    bytes[16..24].copy_from_slice(&10u64.to_le_bytes());
    bytes[24..32].copy_from_slice(&8192u64.to_le_bytes());
    bytes[32..40].copy_from_slice(&132_537_600_000_000_000u64.to_le_bytes());
    bytes[40..44].copy_from_slice(&0x80000100u32.to_le_bytes());
    bytes[56..58].copy_from_slice(&(name.len() as u16).to_le_bytes());
    bytes[58..60].copy_from_slice(&60u16.to_le_bytes());
    bytes[60..60 + name.len()].copy_from_slice(&name);
    let (_, data) = load("sample.usn", &bytes, Default::default());
    assert_eq!(data.kind, "usn");
    let record = data.record(0).unwrap();
    assert_eq!(&record[0], "пример.exe".as_bytes());
    assert!(String::from_utf8_lossy(&record[3]).contains("FILE_CREATE"));
}

#[test]
fn autoruns_compound_versions_dynamic_fields_and_timestamps() {
    use std::io::Write;
    for version in [5u32, 6, 7, 8] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sample.arn");
        let string = |s: &str| {
            let mut data = (s.encode_utf16().count() as u32).to_le_bytes().to_vec();
            data.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
            data
        };
        {
            let mut file = cfb::create(&path).unwrap();
            let mut header = string("Autoruns");
            header.extend(version.to_le_bytes());
            file.create_stream("/Header")
                .unwrap()
                .write_all(&header)
                .unwrap();
            file.create_storage_all("/Items/0").unwrap();
            file.create_stream("/Items/0/Name")
                .unwrap()
                .write_all(&string("пример.exe"))
                .unwrap();
            file.create_stream("/Items/0/ImagePath")
                .unwrap()
                .write_all(&string(r"C:\Пример\пример.exe"))
                .unwrap();
            file.create_stream("/Items/0/NewField")
                .unwrap()
                .write_all(&string("preserved"))
                .unwrap();
            file.create_stream("/Items/0/TimeStamp")
                .unwrap()
                .write_all(&132_537_600_000_000_000u64.to_le_bytes())
                .unwrap();
        }
        let data = Dataset::open(&path, &Default::default(), &Arc::default()).unwrap();
        assert_eq!(data.kind, "arn");
        assert_eq!(data.rows, 1);
        let row = data.record(0).unwrap();
        assert_eq!(&row[0], "пример.exe".as_bytes());
        assert!(data.headers.contains(&"NewField".into()));
        assert!(String::from_utf8_lossy(&row[6]).starts_with("2020-12-30"));
    }
}

#[test]
fn autoruns_unknown_binary_and_derived_column_collisions_preserve_original_fields() {
    use std::io::Write;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("unknown.arn");
    let string = |s: &str| {
        let mut data = (s.encode_utf16().count() as u32).to_le_bytes().to_vec();
        data.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        data
    };
    let (binary, url) = mixed_binary_url();
    {
        let mut file = cfb::create(&path).unwrap();
        let mut header = string("Autoruns");
        header.extend(9u32.to_le_bytes());
        file.create_stream("/Header")
            .unwrap()
            .write_all(&header)
            .unwrap();
        for (id, fields) in [
            (
                "2",
                vec![
                    ("Name", string("first")),
                    ("Category", string("source category")),
                    ("Item", string("source item")),
                    ("Payload", binary.clone()),
                    ("Number", vec![0xff, 0x03, 0x00, 0x01]),
                ],
            ),
            ("10", vec![("Name", string("second"))]),
        ] {
            file.create_storage_all(format!("/Items/{id}")).unwrap();
            for (name, data) in fields {
                file.create_stream(format!("/Items/{id}/{name}"))
                    .unwrap()
                    .write_all(&data)
                    .unwrap();
            }
        }
    }
    let data = Dataset::open(&path, &Default::default(), &Arc::default()).unwrap();
    assert_eq!(data.rows, 2);
    let first = data.record(0).unwrap();
    let at = |name: &str| data.headers.iter().position(|h| h == name).unwrap();
    assert_eq!(&first[at("Name")], b"first");
    assert_eq!(&data.record(1).unwrap()[at("Name")], b"second");
    assert_eq!(&first[at("Category")], b"source category");
    assert_eq!(&first[at("Item")], b"source item");
    assert_eq!(&first[at("Item (inferred 1)")], b"2");
    assert_eq!(&first[at("Payload")], url.as_bytes());
    assert_eq!(&first[at("Number")], b"FF 03 00 01");
    assert_eq!(&first[at("$raw/Number")], b"FF 03 00 01");
    let recovered = String::from_utf8(first[at("$raw/Payload")].to_vec())
        .unwrap()
        .split_whitespace()
        .map(|s| u8::from_str_radix(s, 16).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(recovered, binary);
}

#[test]
fn optional_evtx_exact_comparison_to_reference_parser() {
    use std::collections::BTreeMap;
    let Some(path) = std::env::var_os("EVTX_COMPARE_PATH") else {
        return;
    };
    let started = std::time::Instant::now();
    let data = Dataset::open(
        std::path::Path::new(&path),
        &Default::default(),
        &Arc::default(),
    )
    .unwrap();
    let import_seconds = started.elapsed().as_secs_f64();
    let mut reference = evtx::EvtxParser::from_path(&path)
        .unwrap()
        .with_configuration(evtx::ParserSettings::default().num_threads(1));
    let mut actual = data.reader_at(0).unwrap();
    let mut row = csv::ByteRecord::new();
    fn flatten_reference(
        value: &serde_json::Value,
        path: String,
        fields: &mut BTreeMap<String, String>,
    ) {
        if let serde_json::Value::Object(map) = value
            && !map.is_empty()
        {
            for (name, value) in map {
                flatten_reference(
                    value,
                    format!("{path}/{}", name.replace('~', "~0").replace('/', "~1")),
                    fields,
                );
            }
            return;
        }
        fields.insert(
            path,
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string()),
        );
    }
    let mut index = 0u64;
    for expected in reference.records_json_value() {
        let expected = expected.expect("reference record must be readable");
        let mut fields = BTreeMap::from([
            ("Record ID".into(), expected.event_record_id.to_string()),
            ("Timestamp".into(), expected.timestamp.to_string()),
        ]);
        flatten_reference(&expected.data, String::new(), &mut fields);
        assert!(
            actual.read_byte_record(&mut row).unwrap(),
            "missing row {index}"
        );
        for (column, value) in &fields {
            let position = data.headers.iter().position(|h| h == column).unwrap();
            assert_eq!(&row[position], value.as_bytes(), "row {index}, {column}");
        }
        for (position, column) in data.headers.iter().enumerate() {
            if !fields.contains_key(column) {
                assert!(
                    row.get(position).is_none_or(|s| s.is_empty()),
                    "unexpected row {index}, {column}"
                );
            }
        }
        index += 1;
    }
    assert_eq!(data.rows, index);
    println!(
        "EVTX exact reference comparison: {index} rows, {} columns, import {import_seconds:.3}s; comparison {:.3}s",
        data.headers.len(),
        started.elapsed().as_secs_f64() - import_seconds
    );
}

#[test]
fn optional_real_format_fixtures() {
    for (env, kind) in [
        ("ARN_PATH", "arn"),
        ("EVTX_PATH", "evtx"),
        ("REGISTRY_PATH", "registry"),
    ] {
        if let Some(path) = std::env::var_os(env) {
            let started = std::time::Instant::now();
            let data = Dataset::open(
                std::path::Path::new(&path),
                &Default::default(),
                &Arc::default(),
            )
            .unwrap();
            assert_eq!(data.kind, kind);
            assert!(data.rows > 0);
            assert!(!data.headers.is_empty());
            let _ = data.record(data.rows - 1).unwrap();
            println!(
                "{kind}: rows={} columns={} elapsed={:.3}s warnings={}",
                data.rows,
                data.headers.len(),
                started.elapsed().as_secs_f64(),
                data.warnings.len()
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn live_writable_file_uses_stable_snapshot() {
    use std::{
        io::{Seek, SeekFrom, Write},
        os::windows::fs::OpenOptionsExt,
    };
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("active.csv");
    let mut live = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .share_mode(7)
        .open(&path)
        .unwrap();
    live.write_all(b"Name,Count\none,1\n").unwrap();
    live.flush().unwrap();
    let data = Dataset::open(&path, &Default::default(), &Arc::default()).unwrap();
    assert_eq!(data.acquisition, "shared-snapshot");
    live.seek(SeekFrom::Start(0)).unwrap();
    live.write_all(b"Name,Count\ntwo,2\n").unwrap();
    live.flush().unwrap();
    assert_eq!(&data.record(0).unwrap()[0], b"one");
}
