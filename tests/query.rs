use serde_json::{Value, json};
use std::{
    fs,
    sync::{Arc, mpsc},
    time::Duration,
};
use tempfile::TempDir;
use timeline::{
    bridge::Bridge,
    data::{Dataset, Progress},
    query::{self, ColumnType, Filter, Query, Sort},
};

fn fixture(text: &str) -> (TempDir, Arc<Dataset>) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("table.csv");
    fs::write(&path, text).unwrap();
    let data = Dataset::open(&path, &Default::default(), &Arc::default()).unwrap();
    (dir, data)
}

#[test]
fn language_localizes_only_generated_headers_and_export() {
    use timeline::locale::{self, Language};
    let (dir, data) = fixture("Текст,,Column 2\nНастройки,Загрузка…,Keep\n");
    assert_eq!(data.generated_headers.len(), 1);
    let english = locale::headers(&data, Language::English);
    assert_eq!(english, ["Текст", "Column 2 (2)", "Column 2"]);
    assert_eq!(
        locale::headers(&data, Language::Russian),
        ["Текст", "Столбец 2", "Column 2"]
    );
    let view = sorted(&data, 0, false);
    let path = dir.path().join("export.csv");
    view.export_with_headers(&data, &path, &Progress::default(), &english)
        .unwrap();
    let contents = fs::read_to_string(path).unwrap();
    assert!(contents.starts_with("Текст,Column 2 (2),Column 2"));
    assert!(contents.contains("Настройки,Загрузка…,Keep"));
}

#[test]
fn copying_ranges_uses_sorted_view_full_values_and_column_order() {
    let (dir, _data) = fixture("Name,Count,Details\na,20,\"line1\nline2\"\nb,2,plain\nc,5,end\n");
    let bridge = Arc::new(Bridge::default());
    let call = |request: Value| {
        let (tx, rx) = mpsc::channel();
        bridge.dispatch(request, move |reply| tx.send(reply).unwrap());
        rx.recv_timeout(Duration::from_secs(10)).unwrap()
    };
    let opened = call(json!({"id":1,"command":"open","path":dir.path().join("table.csv")}));
    assert_eq!(opened["ok"], true);
    assert_eq!(
        call(
            json!({"id":2,"command":"query","file":1,"query":{"sort":{"column":1,"descending":false}}})
        )["ok"],
        true
    );
    let copied =
        call(json!({"id":3,"command":"copy_range","revision":2,"start":0,"end":2,"columns":[2,0]}));
    assert_eq!(
        copied["data"]["text"],
        "plain\tb\r\nend\tc\r\n\"line1\nline2\"\ta"
    );
    assert_eq!(
        call(json!({"id":4,"command":"copy_range","revision":2,"start":2,"end":2,"columns":[2]}))["data"]
            ["text"],
        "line1\nline2"
    );
    assert_eq!(
        call(json!({"id":5,"command":"copy_range","revision":1,"start":0,"end":0,"columns":[0]}))["ok"],
        false
    );
}

#[test]
fn source_keeps_original_json_and_unicode_across_chunks() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("source.json");
    let original = "{ \"id\":9007199254740993, \"decimal\":1.2300, \"empty\":null }\r\n";
    fs::write(&path, original).unwrap();
    let bridge = Arc::new(Bridge::default());
    assert_eq!(
        send(&bridge, json!({"id":1,"command":"open","path":path}))["ok"],
        true
    );
    let source = send(&bridge, json!({"id":2,"command":"source","file":1}));
    assert_eq!(source["data"]["text"], original);
    assert_eq!(source["data"]["whole"], true);
    assert_eq!(source["data"]["newline_count"], 1);

    let path = dir.path().join("chunks.txt");
    let prefix = "x".repeat(8 * 1024 * 1024 - 1);
    // The first fragment ends at the newline. A BOM in the next fragment is data.
    let original = format!("{prefix}\n\u{feff}😀Привет\nlast");
    fs::write(&path, &original).unwrap();
    assert_eq!(
        send(&bridge, json!({"id":3,"command":"open","path":path}))["ok"],
        true
    );
    let first = send(&bridge, json!({"id":4,"command":"source","file":3}));
    assert_eq!(first["data"]["whole"], false);
    let second = send(
        &bridge,
        json!({"id":5,"command":"source","file":3,"offset":first["data"]["next"]}),
    );
    assert_eq!(second["data"]["eof"], true);
    assert!(
        second["data"]["text"]
            .as_str()
            .unwrap()
            .starts_with('\u{feff}')
    );
    assert_eq!(
        format!(
            "{}{}",
            first["data"]["text"].as_str().unwrap(),
            second["data"]["text"].as_str().unwrap()
        ),
        original
    );
}

#[test]
fn cell_inspection_decodes_hex_text_without_changing_original() {
    let original = include_str!("fixtures/binary-url.hex").trim();
    let (dir, _data) = fixture(&format!("Payload\n{original}\n"));
    let bridge = Arc::new(Bridge::default());
    assert_eq!(
        send(
            &bridge,
            json!({"id":1,"command":"open","path":dir.path().join("table.csv")})
        )["ok"],
        true
    );
    let cell = send(
        &bridge,
        json!({"id":2,"command":"cell","revision":1,"row":0,"column":0}),
    );
    assert_eq!(cell["data"]["value"], original);
    assert!(
        cell["data"]["decoded"]
            .as_str()
            .unwrap()
            .starts_with("http://example.test/")
    );
    let copy = send(
        &bridge,
        json!({"id":3,"command":"copy_range","revision":1,"start":0,"end":0,"columns":[0]}),
    );
    assert_eq!(copy["data"]["text"], original);
}
fn sorted(data: &Dataset, column: usize, descending: bool) -> query::View {
    query::execute(
        data,
        &Query {
            sort: Some(Sort {
                column,
                descending,
                kind: None,
            }),
            ..Default::default()
        },
        &query::infer_types(data).unwrap(),
        &Progress::default(),
    )
    .unwrap()
}

#[test]
fn numbers_dates_timezones_empty_and_stable_ties() {
    let (_dir, data) = fixture(
        "Name,Count,Created\nfirst,100,31.12.2025\nsecond,2,02.01.2026\nthird,-3,2026-01-01T23:00:00-02:00\nfourth,2,2026-01-02T00:30:00Z\nempty,,\n",
    );
    assert_eq!(
        query::infer_types(&data).unwrap(),
        [ColumnType::Text, ColumnType::Number, ColumnType::Date]
    );
    let ids = |view: query::View| {
        view.records(&data, 0, 20)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(sorted(&data, 1, false)), [2, 1, 3, 0, 4]);
    assert_eq!(ids(sorted(&data, 1, true)), [0, 1, 3, 2, 4]);
    assert_eq!(ids(sorted(&data, 2, false)), [0, 1, 3, 2, 4]);
    assert_eq!(ids(sorted(&data, 2, true)), [2, 3, 1, 0, 4]);
    assert!(query::date("31.02.2026").is_none());
    assert_eq!(query::date("03/04/2026"), query::date("2026-04-03"));
    assert_eq!(query::number("1\u{a0}234,50"), Some(1234.5));
    assert_eq!(
        query::date("2026-09-23 08:00:00 UTC"),
        query::date("2026-09-23T08:00:00Z")
    );
}

#[test]
fn integer_ids_preserve_precision_and_decimal_order() {
    let (_dir, data) = fixture(
        "Name,ID\na,9007199254740993\nb,9007199254740992\nc,1.20001\nd,1.2\ne,-0.002\nf,-10\ng,1.2e1\nh,0\n",
    );
    let view = sorted(&data, 1, false);
    assert_eq!(
        view.records(&data, 0, 8)
            .unwrap()
            .iter()
            .map(|x| x.0)
            .collect::<Vec<_>>(),
        [5, 4, 7, 3, 2, 6, 1, 0]
    );
}

#[test]
fn russian_alphabet_and_natural_numbers() {
    let (_dir, data) = fixture("Name\nЯна\nЁж\nЕгор\nАня\nборис\nфайл10\nфайл2\nеж\n");
    let view = sorted(&data, 0, false);
    assert_eq!(
        view.records(&data, 0, 8)
            .unwrap()
            .iter()
            .map(|x| x.0)
            .collect::<Vec<_>>(),
        [3, 4, 2, 7, 1, 6, 5, 0]
    );
}

#[test]
fn dates_keep_submicrosecond_precision_and_historical_years() {
    let (_dir, data) = fixture(
        "Name,Date\nlater,2026-01-01T00:00:00.000000002Z\nearlier,2026-01-01T00:00:00.000000001Z\nhistorical,1601-01-01T00:00:00Z\n",
    );
    assert_eq!(
        sorted(&data, 1, false)
            .records(&data, 0, 3)
            .unwrap()
            .iter()
            .map(|x| x.0)
            .collect::<Vec<_>>(),
        [2, 1, 0]
    );
}

#[test]
fn search_and_filters_cover_late_rows_and_entire_cell_not_preview() {
    let mut csv = String::from("Name,Count,Details\n");
    for row in 0..120_257 {
        csv.push_str(&format!(
            "row-{row},{row},{}\n",
            if row == 120_256 {
                format!("{}ИГОЛКА", "x".repeat(4000))
            } else {
                "обычный текст".into()
            }
        ));
    }
    let (_dir, data) = fixture(&csv);
    let types = query::infer_types(&data).unwrap();
    let query = Query {
        text: "иголка".into(),
        filters: vec![Filter {
            column: 1,
            op: "equals".into(),
            text: "120256".into(),
        }],
        sort: Some(Sort {
            column: 1,
            descending: true,
            kind: None,
        }),
        ..Default::default()
    };
    let result = query::execute(&data, &query, &types, &Progress::default()).unwrap();
    assert_eq!(result.rows, 1);
    let rows = result.viewport(&data, 0, 100, &[0, 2]).unwrap();
    assert_eq!(rows[0].id, 120_256);
    assert_eq!(rows[0].values[0], "row-120256");
    assert!(rows[0].values[1].len() < 600);
    let all = sorted(&data, 1, true);
    assert_eq!(all.rows, 120_257);
    assert_eq!(
        all.viewport(&data, 120_256, 1, &[0]).unwrap()[0].values[0],
        "row-0"
    );
}

#[test]
fn sorted_random_offsets_handle_multiline_records_and_export_order() {
    let (_dir, data) = fixture(
        "Name,Count,Details\nalpha,20,\"first\nsecond, with comma\"\nbeta,3,\"quotes \"\"here\"\"\"\ngamma,1,end\n",
    );
    let view = sorted(&data, 1, false);
    assert_eq!(
        view.records(&data, 2, 1).unwrap()[0].1[2],
        b"first\nsecond, with comma"[..]
    );
    let out = TempDir::new().unwrap();
    let path = out.path().join("sorted.csv");
    view.export(&data, &path, &Progress::default()).unwrap();
    let reopened = Dataset::open(&path, &Default::default(), &Arc::default()).unwrap();
    assert_eq!(&reopened.record(0).unwrap()[0], b"gamma");
    assert_eq!(&reopened.record(2).unwrap()[0], b"alpha");
    assert!(
        view.export(&data, &data.original_path, &Progress::default())
            .is_err()
    );
    let alias = data
        .original_path
        .parent()
        .unwrap()
        .join(".")
        .join("table.csv");
    let error = view
        .export(&data, &alias, &Progress::default())
        .unwrap_err();
    assert!(error.to_string().contains("Нельзя перезаписать"));
}

#[test]
fn empty_invalid_and_cancelled_queries() {
    let (_dir, data) = fixture("Name,Date\n");
    assert_eq!(sorted(&data, 1, true).rows, 0);
    assert!(
        sorted(&data, 1, true)
            .viewport(&data, 0, 10, &[0])
            .unwrap()
            .is_empty()
    );
    assert!(
        query::execute(
            &data,
            &Query {
                regex: true,
                text: "[".into(),
                ..Default::default()
            },
            &[ColumnType::Text; 2],
            &Progress::default()
        )
        .is_err()
    );
    let progress = Progress::default();
    progress.cancel();
    assert!(query::execute(&data, &Query::default(), &[], &progress).is_err());
}

#[test]
fn timestamp_units_and_explicit_date_override() {
    let (_dir, data) = fixture(
        "Name,UnixTimestamp,Event\na,1767225600000,02.01.2026\nb,1767225599,unknown\nc,1767225600000001,01.01.2026\n",
    );
    let types = query::infer_types(&data).unwrap();
    assert_eq!(types[1], ColumnType::Timestamp);
    assert_eq!(types[2], ColumnType::Text);
    assert_eq!(
        sorted(&data, 1, false)
            .records(&data, 0, 3)
            .unwrap()
            .iter()
            .map(|x| x.0)
            .collect::<Vec<_>>(),
        [1, 0, 2]
    );
    let view = query::execute(
        &data,
        &Query {
            sort: Some(Sort {
                column: 2,
                descending: true,
                kind: Some(ColumnType::Date),
            }),
            ..Default::default()
        },
        &types,
        &Progress::default(),
    )
    .unwrap();
    assert_eq!(
        view.records(&data, 0, 3)
            .unwrap()
            .iter()
            .map(|x| x.0)
            .collect::<Vec<_>>(),
        [0, 2, 1]
    );
}

fn send(bridge: &Arc<Bridge>, request: Value) -> Value {
    let (tx, rx) = mpsc::channel();
    bridge.dispatch(request, move |v| {
        tx.send(v).unwrap();
    });
    rx.recv_timeout(Duration::from_secs(20)).unwrap()
}
#[test]
fn ipc_revisions_reject_stale_rows_and_failed_query_preserves_current_view() {
    let (dir, _data) = fixture("Name,Date\na,02.01.2026\nb,01.01.2026\n");
    let bridge = Arc::new(Bridge::default());
    assert_eq!(
        send(
            &bridge,
            json!({"id":1,"command":"open","path":dir.path().join("table.csv")})
        )["ok"],
        true
    );
    assert_eq!(
        send(
            &bridge,
            json!({"id":2,"command":"query","file":1,"query":{"sort":{"column":1,"descending":false}}})
        )["data"]["rows"],
        2
    );
    assert_eq!(
        send(
            &bridge,
            json!({"id":3,"command":"rows","revision":1,"start":0,"count":1,"columns":[0]})
        )["ok"],
        false
    );
    let rows = send(
        &bridge,
        json!({"id":4,"command":"rows","revision":2,"start":0,"count":1,"columns":[0]}),
    );
    assert_eq!(rows["data"]["rows"][0]["values"][0], "b");
    assert_eq!(
        send(
            &bridge,
            json!({"id":5,"command":"query","file":1,"query":{"text":"[","regex":true}})
        )["ok"],
        false
    );
    assert_eq!(
        send(
            &bridge,
            json!({"id":6,"command":"rows","revision":2,"start":0,"count":1,"columns":[0]})
        )["ok"],
        true
    );
}
