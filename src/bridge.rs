//! Asynchronous IPC API. Every view has a revision to reject stale viewport requests.
use crate::{
    data::{Dataset, Encoding, Format, OpenOptionsConfig, Progress, preview},
    locale::{self, Language},
    query::{self, ColumnType, View},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

struct State {
    data: Arc<Dataset>,
    types: Vec<ColumnType>,
    view: Arc<View>,
    file: u64,
    revision: u64,
}
struct Job {
    id: u64,
    progress: Arc<Progress>,
    label: &'static str,
    started: Instant,
}
#[derive(Default)]
pub struct Bridge {
    state: Mutex<Option<State>>,
    job: Mutex<Option<Job>>,
}

impl Bridge {
    pub fn dispatch(self: &Arc<Self>, request: Value, reply: impl Fn(Value) + Send + 'static) {
        let id = request["id"].as_u64().unwrap_or(0);
        let command = request["command"].as_str().unwrap_or("").to_owned();
        if command == "cancel" {
            if let Some(job) = self.job.lock().unwrap().as_ref() {
                job.progress.cancel();
            }
            reply(json!({"id": id, "ok": true, "data": null}));
            return;
        }
        let progress = Arc::new(Progress::default());
        let is_job = matches!(command.as_str(), "open" | "query" | "export");
        if is_job {
            let mut slot = self.job.lock().unwrap();
            if let Some(job) = slot.take() {
                job.progress.cancel();
            }
            *slot = Some(Job {
                id,
                progress: progress.clone(),
                started: Instant::now(),
                label: match command.as_str() {
                    "open" => "open",
                    "export" => "export",
                    _ => "query",
                },
            });
        }
        let bridge = self.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                bridge.process(&request, &progress)
            }))
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "Parser failed on this file. Try opening it as raw bytes."
                ))
            });
            if is_job {
                let mut slot = bridge.job.lock().unwrap();
                if slot.as_ref().is_some_and(|job| job.id == id) {
                    *slot = None;
                }
            }
            reply(match result {
                Ok(data) => json!({"id": id, "ok": true, "data": data}),
                Err(error) => {
                    // Workers also set the cancellation flag to stop siblings after
                    // an I/O/parser failure. Preserve that failure in the UI.
                    let cancelled = error.chain().any(|cause| {
                        matches!(
                            cause.to_string().as_str(),
                            "Операция отменена" | "Операция устарела"
                        )
                    });
                    json!({"id": id, "ok": false, "error": format!("{error:#}"), "error_en": locale::english_error(&error), "needs_elevation":error.downcast_ref::<crate::native::NeedsElevation>().is_some(), "can_open_raw": command == "open" && request["path"].as_str().is_some_and(|p| crate::data::open_read(std::path::Path::new(p)).is_ok()), "cancelled": cancelled})
                }
            });
        });
    }
    pub fn progress(&self) -> Option<Value> {
        self.job.lock().unwrap().as_ref().map(|job| {
            let mut snapshot = job.progress.snapshot();
            snapshot["event"] = json!("progress");
            snapshot["id"] = json!(job.id);
            snapshot["kind"] = json!(job.label);
            snapshot["elapsed"] = json!(job.started.elapsed().as_secs_f64());
            snapshot
        })
    }
    fn process(&self, request: &Value, progress: &Arc<Progress>) -> Result<Value> {
        let id = request["id"].as_u64().context("Нет номера запроса")?;
        let language = Language::from_code(request["language"].as_str());
        match request["command"].as_str().unwrap_or("") {
            "pick" => Ok(json!(
                rfd::FileDialog::new()
                    .set_title(language.text("Открыть таблицу", "Open table"))
                    .add_filter(
                        language.text("Таблицы и журналы", "Tables and logs"),
                        &[
                            "csv", "tsv", "psv", "json", "jsonl", "ndjson", "txt", "log", "md",
                            "js", "xml", "evtx", "edb", "dat", "arn", "sqlite", "db"
                        ]
                    )
                    .add_filter(language.text("Все файлы", "All files"), &["*"])
                    .pick_file()
                    .map(|p| p.to_string_lossy().into_owned())
            )),
            "open" => {
                let path = PathBuf::from(request["path"].as_str().context("Выберите файл")?);
                let options = &request["options"];
                let config = OpenOptionsConfig {
                    auto_text: options["autoText"].as_bool().unwrap_or(true),
                    header: options["header"].as_bool().unwrap_or(true),
                    delimiter: options["delimiter"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(|s| s.as_bytes()[0]),
                    format: match options["format"].as_str() {
                        Some("csv") => Format::Csv,
                        Some("json") => Format::Json,
                        Some("text") => Format::Text,
                        Some("hex") => Format::Hex,
                        _ => Format::Auto,
                    },
                    encoding: match options["encoding"].as_str() {
                        Some("utf8") => Encoding::Utf8,
                        Some("utf16le") => Encoding::Utf16Le,
                        Some("utf16be") => Encoding::Utf16Be,
                        Some("1251") => Encoding::Windows1251,
                        Some("1252") => Encoding::Windows1252,
                        _ => Encoding::Auto,
                    },
                };
                let started = Instant::now();
                let data = Dataset::open(&path, &config, progress)?;
                progress.begin("prepare", data.rows.min(640), "records");
                let types = query::infer_types_with_progress(&data, progress)?;
                let result = json!({"file": id, "revision": id, "name": path.file_name().unwrap_or_default().to_string_lossy(), "path": path.to_string_lossy(), "headers": data.headers, "generated_headers": data.generated_headers, "types": types, "total": data.rows, "rows": data.rows, "bytes": data.bytes, "description": data.description, "kind":data.kind,"warnings":data.warnings,"acquisition":data.acquisition, "irregular": data.irregular_rows, "elapsed": started.elapsed().as_secs_f64()});
                let slot = self.job.lock().unwrap();
                ensure!(self.is_current(&slot, id), "Операция устарела");
                progress.check()?;
                *self.state.lock().unwrap() = Some(State {
                    view: Arc::new(View {
                        rows: data.rows,
                        index: None,
                    }),
                    data,
                    types,
                    file: id,
                    revision: id,
                });
                Ok(result)
            }
            "query" => {
                let (data, types, file) = {
                    let state = self.state.lock().unwrap();
                    let state = state.as_ref().context("Сначала откройте файл")?;
                    ensure!(
                        request["file"].as_u64() == Some(state.file),
                        "Файл уже изменился"
                    );
                    (state.data.clone(), state.types.clone(), state.file)
                };
                let query = serde_json::from_value(request["query"].clone())?;
                let started = Instant::now();
                let view = Arc::new(query::execute(&data, &query, &types, progress)?);
                let result = json!({"revision": id, "rows": view.rows, "elapsed": started.elapsed().as_secs_f64()});
                let slot = self.job.lock().unwrap();
                ensure!(self.is_current(&slot, id), "Операция устарела");
                progress.check()?;
                let mut state = self.state.lock().unwrap();
                let state = state.as_mut().context("Файл закрыт")?;
                ensure!(state.file == file, "Файл уже изменился");
                state.view = view;
                state.revision = id;
                Ok(result)
            }
            "elevate" => {
                let path = PathBuf::from(request["path"].as_str().context("Выберите файл")?);
                crate::native::relaunch_elevated(&path, &request["settings"].to_string())?;
                Ok(Value::Null)
            }
            "source" => {
                use std::io::{Read, Seek, SeekFrom};
                let data = {
                    let slot = self.state.lock().unwrap();
                    let state = slot.as_ref().context("Сначала откройте файл")?;
                    ensure!(
                        request["file"].as_u64() == Some(state.file),
                        "Файл уже изменился"
                    );
                    state.data.clone()
                };
                let offset = request["offset"].as_u64().unwrap_or(0);
                let mut input = crate::data::open_read(&data.source_path)?;
                let length = input.metadata()?.len();
                ensure!(offset <= length, "Invalid source offset");
                input.seek(SeekFrom::Start(offset))?;
                let mut bytes = Vec::new();
                input.take(8 * 1024 * 1024).read_to_end(&mut bytes)?;
                let mut consumed = bytes.len();
                if offset + (consumed as u64) < length {
                    if let Some(newline) = bytes
                        .iter()
                        .rposition(|&b| b == b'\n')
                        .filter(|&i| i > bytes.len() / 2)
                    {
                        consumed = newline + 1;
                    } else if let Err(e) = std::str::from_utf8(&bytes)
                        && e.error_len().is_none()
                    {
                        consumed = e.valid_up_to();
                    }
                }
                let text = std::str::from_utf8(&bytes[..consumed])
                    .context("Source is binary; use the hexadecimal table")?;
                let text = if offset == 0 {
                    text.strip_prefix('\u{feff}').unwrap_or(text)
                } else {
                    text
                };
                let eof = offset + consumed as u64 >= length;
                let newline_count = memchr::memchr_iter(b'\n', text.as_bytes()).count();
                Ok(
                    json!({"text":text,"offset":offset,"next":offset+consumed as u64,"eof":eof,"whole":offset==0 && eof,"newline_count":newline_count,"bytes":length,"kind":data.kind}),
                )
            }
            "rows" | "cell" | "copy_range" | "export" => {
                let (data, view) = {
                    let state = self.state.lock().unwrap();
                    let state = state.as_ref().context("Сначала откройте файл")?;
                    ensure!(
                        request["revision"].as_u64() == Some(state.revision),
                        "Представление уже изменилось"
                    );
                    (state.data.clone(), state.view.clone())
                };
                match request["command"].as_str().unwrap() {
                    "copy_range" => {
                        let start = request["start"].as_u64().context("Строка недоступна")?;
                        let end = request["end"].as_u64().context("Строка недоступна")?;
                        let columns: Vec<usize> =
                            serde_json::from_value(request["columns"].clone())?;
                        ensure!(start <= end && end < view.rows, "Строка недоступна");
                        ensure!(
                            !columns.is_empty() && columns.iter().all(|c| *c < data.headers.len()),
                            "Столбец недоступен"
                        );
                        if start == end && columns.len() == 1 {
                            let record = view
                                .records(&data, start, 1)?
                                .pop()
                                .context("Строка недоступна")?
                                .1;
                            return Ok(
                                json!({"text":std::str::from_utf8(record.get(columns[0]).unwrap_or_default())?}),
                            );
                        }
                        ensure!(
                            (end - start + 1).saturating_mul(columns.len() as u64) <= 1_000_000,
                            "Слишком большое выделение. Используйте экспорт CSV"
                        );
                        let mut writer = csv::WriterBuilder::new()
                            .delimiter(b'\t')
                            .terminator(csv::Terminator::CRLF)
                            .from_writer(Vec::new());
                        for first in (start..=end).step_by(256) {
                            progress.check()?;
                            for (_, record) in
                                view.records(&data, first, (end - first + 1).min(256) as usize)?
                            {
                                writer.write_record(
                                    columns.iter().map(|c| record.get(*c).unwrap_or_default()),
                                )?;
                            }
                            writer.flush()?;
                            ensure!(
                                writer.get_ref().len() <= 64 * 1024 * 1024,
                                "Слишком большое выделение. Используйте экспорт CSV"
                            );
                        }
                        let mut text = String::from_utf8(writer.into_inner()?)?;
                        text.truncate(text.len().saturating_sub(2));
                        Ok(json!({"text":text}))
                    }
                    "rows" => {
                        let start = request["start"].as_u64().unwrap_or(0);
                        let count = request["count"].as_u64().unwrap_or(100).min(256) as usize;
                        let columns: Vec<usize> =
                            serde_json::from_value(request["columns"].clone())?;
                        Ok(
                            json!({"start": start, "rows": view.viewport(&data, start, count, &columns)?}),
                        )
                    }
                    "cell" => {
                        let row = request["row"].as_u64().context("Строка недоступна")?;
                        let column =
                            request["column"].as_u64().context("Столбец недоступен")? as usize;
                        ensure!(column < data.headers.len(), "Столбец недоступен");
                        let record = data.record(row)?;
                        let value = std::str::from_utf8(record.get(column).unwrap_or_default())
                            .unwrap_or("");
                        let truncated = value.chars().count() > 1_000_000;
                        let decoded = if truncated {
                            None
                        } else {
                            crate::binary::readable_hex(value)
                        };
                        Ok(
                            json!({"value": preview(value, 1_000_000), "truncated": truncated,"decoded":decoded}),
                        )
                    }
                    _ => {
                        let path = rfd::FileDialog::new()
                            .set_title(language.text("Сохранить результат", "Save results"))
                            .set_file_name("timeline-export.csv")
                            .add_filter("CSV", &["csv"])
                            .save_file();
                        if let Some(path) = path {
                            progress.check()?;
                            Ok(
                                json!({"rows": view.export_with_headers(&data, &path, progress, &locale::headers(&data, language))?, "path": path.to_string_lossy()}),
                            )
                        } else {
                            Ok(Value::Null)
                        }
                    }
                }
            }
            _ => bail!("Неизвестная команда"),
        }
    }
    fn is_current(&self, job: &Option<Job>, id: u64) -> bool {
        job.as_ref().is_some_and(|job| job.id == id)
    }
}
