use std::{fs, sync::Arc};
use tempfile::TempDir;
use timeline::{
    data::{Dataset, Encoding, Format, OpenOptionsConfig, Progress, STRIDE, detect_delimiter},
    search::{self, FilterOp, FilterRule, FindSpec, ScanRequest, Scope},
};

fn fixture(name: &str, bytes: &[u8], options: OpenOptionsConfig) -> (TempDir, Arc<Dataset>) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, bytes).unwrap();
    let data = Dataset::open(&path, &options, &Arc::default()).unwrap();
    (dir, data)
}
fn find(
    data: &Dataset,
    text: &str,
    regex: bool,
    sensitive: bool,
    scope: Scope,
) -> search::SearchOutput {
    search::scan(
        data,
        &ScanRequest {
            find: Some(FindSpec {
                text: text.into(),
                regex,
                case_sensitive: sensitive,
                scope,
            }),
            ..Default::default()
        },
        &Arc::default(),
    )
    .unwrap()
}

#[test]
fn quoted_multiline_csv_bom_empty_fields_and_crlf() {
    let (_, data) = fixture(
        "test.csv",
        b"\xef\xbb\xbfID,Text,Last\r\n1,\"hello, \"\"world\"\"\r\nline two\",\r\n2,,end",
        Default::default(),
    );
    assert_eq!(data.headers, ["ID", "Text", "Last"]);
    assert_eq!(data.rows, 2);
    assert_eq!(&data.record(0).unwrap()[1], b"hello, \"world\"\r\nline two");
    assert_eq!(&data.record(0).unwrap()[2], b"");
    assert_eq!(&data.record(1).unwrap()[2], b"end");
}

#[test]
fn sparse_seeks_and_parallel_scans_preserve_record_boundaries() {
    let mut text = String::from("id,text\n");
    for n in 0..10_257 {
        text.push_str(&format!("{n},\"prefix\nMARKER {n}, suffix\"\r\n"));
    }
    let (_dir, data) = fixture("records.csv", text.as_bytes(), Default::default());
    assert_eq!(data.checkpoints.len() as u64, data.rows.div_ceil(STRIDE));
    for n in [0, 127, 128, 511, 512, 513, 4096, 10256] {
        let row = data.record(n).unwrap();
        assert_eq!(&row[0], n.to_string().as_bytes());
    }
    let output = find(&data, "MARKER", false, true, Scope::Table);
    assert_eq!(output.hits, data.rows);
    assert_eq!(output.matched_rows.len(), data.rows);
    let hits = output.hit_page(0, data.rows).unwrap();
    for (n, hit) in hits.iter().enumerate() {
        assert_eq!(hit.row, n as u64);
        assert_eq!(hit.column, 1);
        assert_eq!(hit.start, 7);
    }
    assert_eq!(output.hit_page(10_255, 5).unwrap().len(), 2);
}

#[test]
fn unicode_case_insensitive_literal_regex_and_scopes() {
    let (_dir, data) = fixture(
        "table.csv",
        "name,details\nПРОЦЕСС,Процесс процесс\nother,ПРОЦЕСС\n".as_bytes(),
        Default::default(),
    );
    assert_eq!(find(&data, "процесс", false, false, Scope::Table).hits, 4);
    assert_eq!(find(&data, "процесс", false, true, Scope::Table).hits, 1);
    assert_eq!(
        find(&data, "процесс", false, false, Scope::Column(0)).hits,
        1
    );
    let result = find(
        &data,
        "процесс",
        false,
        false,
        Scope::Cell { row: 0, column: 1 },
    );
    assert_eq!(result.hits, 2);
    assert_eq!(result.hit_page(1, 1).unwrap()[0].start, 15);
    assert_eq!(find(&data, "^процесс$", true, false, Scope::Table).hits, 2);
    assert!(
        search::compile_pattern(&FindSpec {
            text: "[".into(),
            regex: true,
            case_sensitive: false,
            scope: Scope::Table
        })
        .is_err()
    );
}

#[test]
fn search_does_not_cross_cells_and_handles_zero_width_matches() {
    let (_dir, data) = fixture("data.csv", b"a,b\nx,y\nx.x,x\n", Default::default());
    assert_eq!(find(&data, "xy", false, true, Scope::Table).hits, 0);
    assert_eq!(find(&data, ".", false, true, Scope::Table).hits, 1);
    assert_eq!(find(&data, "^", true, true, Scope::Table).hits, 4);
}

#[test]
fn filters_and_restricted_search() {
    let (_dir, data) = fixture(
        "f.csv",
        b"name,state\nAlpha,ok\nALPHA,bad\nBeta,\nGamma\n",
        Default::default(),
    );
    let filter = |col, op, text: &str| FilterRule {
        column: col,
        op,
        text: text.into(),
        case_sensitive: false,
    };
    let out = search::scan(
        &data,
        &ScanRequest {
            filters: vec![
                filter(0, FilterOp::Equals, "alpha"),
                filter(1, FilterOp::NotEquals, "bad"),
            ],
            ..Default::default()
        },
        &Arc::default(),
    )
    .unwrap();
    assert_eq!(out.matched_rows.iter().collect::<Vec<_>>(), [0]);
    let result = search::scan(
        &data,
        &ScanRequest {
            find: Some(FindSpec {
                text: "a".into(),
                regex: false,
                case_sensitive: false,
                scope: Scope::Table,
            }),
            restrict: Some(out.matched_rows.clone()),
            ..Default::default()
        },
        &Arc::default(),
    )
    .unwrap();
    assert_eq!(result.hits, 2);
    let empty = search::scan(
        &data,
        &ScanRequest {
            filters: vec![filter(1, FilterOp::Empty, "")],
            ..Default::default()
        },
        &Arc::default(),
    )
    .unwrap();
    assert_eq!(empty.matched_rows.iter().collect::<Vec<_>>(), [2, 3]);
}

#[test]
fn delimiter_headerless_and_ragged_rows() {
    assert_eq!(detect_delimiter(b"a;b;c\n1;\"x,y\";3\n"), Some(b';'));
    assert_eq!(detect_delimiter(b"a\tb\n1\t2\n"), Some(b'\t'));
    let (_dir, data) = fixture(
        "ragged.csv",
        b"a,a,a (2)\n1,2\n3,4,5,6\n",
        Default::default(),
    );
    assert_eq!(data.headers.len(), 4);
    assert_eq!(data.irregular_rows, 2);
    assert_eq!(
        data.headers
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        4
    );
    let (_dir2, data2) = fixture(
        "headerless.tsv",
        b"1\t2\n3\t4\n",
        OpenOptionsConfig {
            header: false,
            ..Default::default()
        },
    );
    assert_eq!(data2.rows, 2);
    assert_eq!(&data2.record(0).unwrap()[0], b"1");
}

#[test]
fn json_array_late_columns_nested_objects_and_empty_array() {
    let (_dir, data) = fixture("a.json", br#"[{"id":1,"user":{"name":"a"},"a/b":"literal"},{"id":2,"late":"found","user":{"name":"b"}}]"#, Default::default());
    assert_eq!(data.rows, 2);
    let column = data.headers.iter().position(|h| h == "/late").unwrap();
    assert!(data.headers.contains(&"/user/name".into()));
    assert!(data.headers.contains(&"/a~1b".into()));
    assert_eq!(data.record(0).unwrap().get(column).unwrap_or(b""), b"");
    assert_eq!(&data.record(1).unwrap()[column], b"found");
    assert_eq!(find(&data, "found", false, true, Scope::Table).hits, 1);
    let (_dir2, data2) = fixture("empty.json", b"[]", Default::default());
    assert_eq!(data2.rows, 0);
}

#[test]
fn json_lines_bom_scalars_and_malformed_input() {
    let (_dir, data) = fixture(
        "a.jsonl",
        b"\xef\xbb\xbf{\"name\":\"one\"}\n{\"name\":\"two\",\"n\":null}\n",
        Default::default(),
    );
    assert_eq!(data.rows, 2);
    assert_eq!(find(&data, "null", false, true, Scope::Table).hits, 1);
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bad.json");
    fs::write(&path, b"[{\"a\":1},]").unwrap();
    assert!(Dataset::open(&path, &Default::default(), &Arc::default()).is_err());
}

#[test]
fn text_preserves_empty_lines_and_quoted_text() {
    let (_dir, data) = fixture(
        "a.txt",
        b"one, \"quoted\"\r\n\r\nlast",
        OpenOptionsConfig {
            format: Format::Text,
            ..Default::default()
        },
    );
    assert_eq!(data.rows, 3);
    assert_eq!(&data.record(0).unwrap()[0], b"one, \"quoted\"");
    assert_eq!(&data.record(1).unwrap()[0], b"");
}

#[test]
fn utf16_and_windows1251() {
    let text = "name,value\r\nПроцесс,данные\r\n";
    let bytes: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    let (_dir, data) = fixture("u.csv", &bytes, Default::default());
    assert_eq!(&data.record(0).unwrap()[0], "Процесс".as_bytes());
    let (encoded, _, _) = encoding_rs::WINDOWS_1251.encode(text);
    let (_dir2, data2) = fixture(
        "ansi.csv",
        &encoded,
        OpenOptionsConfig {
            encoding: Encoding::Windows1251,
            ..Default::default()
        },
    );
    assert_eq!(find(&data2, "ДАННЫЕ", false, false, Scope::Table).hits, 1);
}

#[test]
fn cancelled_jobs_empty_files_and_export_roundtrip() {
    let (_dir, empty) = fixture("empty.csv", b"a,b\n", Default::default());
    assert_eq!(find(&empty, "x", false, true, Scope::Table).hits, 0);
    let (_dir2, data) = fixture(
        "data.csv",
        b"a,b\n1,\"hello, world\"\n2,test\n",
        Default::default(),
    );
    let progress = Arc::new(Progress::default());
    progress.cancel();
    assert!(search::scan(&data, &ScanRequest::default(), &progress).is_err());
    let result = find(&data, "hello", false, true, Scope::Table);
    let out_dir = TempDir::new().unwrap();
    let out = out_dir.path().join("out.csv");
    assert_eq!(
        search::export_csv(
            &data,
            Some(&result.matched_rows),
            &out,
            &Progress::default()
        )
        .unwrap(),
        1
    );
    let reopened = Dataset::open(&out, &Default::default(), &Arc::default()).unwrap();
    assert_eq!(reopened.rows, 1);
    assert_eq!(reopened.record(0).unwrap(), data.record(0).unwrap());
    assert!(search::export_csv(&data, None, &data.original_path, &Progress::default()).is_err());
}

#[cfg(windows)]
#[test]
fn open_dataset_prevents_writes_to_original() {
    let (_dir, data) = fixture("lock.csv", b"a\n1\n", Default::default());
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&data.original_path)
            .is_err()
    );
}

#[test]
fn find_beyond_preview_limit() {
    let text = format!("data\n{}needle needle\n", "x".repeat(20_000));
    let (_dir, data) = fixture("long.csv", text.as_bytes(), Default::default());
    assert!(data.page(0).unwrap()[0][0].len() < 1100);
    let result = find(
        &data,
        "needle",
        false,
        true,
        Scope::Cell { row: 0, column: 0 },
    );
    assert_eq!(result.hits, 2);
    assert_eq!(result.hit_page(0, 1).unwrap()[0].start, 20_000);
}

#[test]
fn malformed_csv_quotes_and_broken_utf16_are_rejected() {
    for bytes in [
        b"a,b\n1,\"unterminated\n2,value\n".as_slice(),
        b"a,b\n1,\"text\"invalid\n".as_slice(),
    ] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("broken.csv");
        fs::write(&path, bytes).unwrap();
        assert!(Dataset::open(&path, &Default::default(), &Arc::default()).is_err());
    }
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("broken.csv");
    fs::write(&path, [0xFF, 0xFE, b'a']).unwrap();
    assert!(Dataset::open(&path, &Default::default(), &Arc::default()).is_err());
}

#[test]
fn quoted_record_spans_io_buffer_and_bom_inside_text_is_preserved() {
    let long = format!(
        "name,value\r\nfirst,\"{}\"\"text\"\r\nlast,end\r\n",
        "long ".repeat(70_000)
    );
    let (_dir, data) = fixture("wide.csv", long.as_bytes(), Default::default());
    assert_eq!(data.rows, 2);
    assert_eq!(&data.record(1).unwrap()[0], b"last");
    let (_dir2, text) = fixture(
        "t.txt",
        "\u{feff}first\n\u{feff}second\n".as_bytes(),
        OpenOptionsConfig {
            format: Format::Text,
            ..Default::default()
        },
    );
    assert_eq!(&text.record(1).unwrap()[0], "\u{feff}second".as_bytes());
}

#[test]
fn parallel_index_preserves_multiline_records_and_sparse_seeks() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("parallel.csv");
    let payload = format!("{}\nquoted \"text\"\nend", "x".repeat(730));
    {
        let mut writer = csv::Writer::from_path(&path).unwrap();
        writer.write_record(["id", "text", "literal"]).unwrap();
        for row in 0..50_003 {
            writer
                .write_record([row.to_string(), payload.clone(), "unquoted value".into()])
                .unwrap();
        }
        writer
            .write_record(["last", &"z".repeat(9 * 1024 * 1024), "end"])
            .unwrap();
        writer.flush().unwrap();
    }
    let data = Dataset::open(&path, &Default::default(), &Arc::default()).unwrap();
    assert_eq!(data.rows, 50_004);
    for row in [0, 511, 512, 10_000, 22_999, 50_002] {
        let record = data.record(row).unwrap();
        assert_eq!(&record[0], row.to_string().as_bytes());
        assert_eq!(&record[1], payload.as_bytes());
    }
    assert_eq!(&data.record(50_003).unwrap()[0], b"last");
    drop(data);
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"broken,\"unterminated")
        .unwrap();
    assert!(Dataset::open(&path, &Default::default(), &Arc::default()).is_err());
}

#[test]
fn parallel_index_treats_bom_inside_data_as_text() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bom-data.csv");
    use std::io::Write;
    let mut file = fs::File::create(&path).unwrap();
    file.write_all("name,text\n\u{feff}\"literal,first\nlast,".as_bytes())
        .unwrap();
    file.write_all(&vec![b'x'; 34 * 1024 * 1024]).unwrap();
    file.write_all(b"\n").unwrap();
    drop(file);
    let data = Dataset::open(&path, &Default::default(), &Arc::default()).unwrap();
    assert_eq!(data.rows, 2);
    assert_eq!(&data.record(0).unwrap()[0], "\u{feff}\"literal".as_bytes());
    assert_eq!(&data.record(1).unwrap()[0], b"last");
}
