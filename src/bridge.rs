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
                label: match command.as_str() {
                    "open" => "open",
                    "export" => "export",
                    _ => "query",
                },
            });
        }
        let bridge = self.clone();
        std::thread::spawn(move || {
            let result = bridge.process(&request, &progress);
            if is_job {
                let mut slot = bridge.job.lock().unwrap();
                if slot.as_ref().is_some_and(|job| job.id == id) {
                    *slot = None;
                }
            }
            reply(match result {
                Ok(data) => json!({"id": id, "ok": true, "data": data}),
                Err(error) => {
                    json!({"id": id, "ok": false, "error": format!("{error:#}"), "error_en": locale::english_error(&error), "cancelled": matches!(error.to_string().as_str(), "Операция отменена" | "Операция устарела")})
                }
            });
        });
    }
    pub fn progress(&self) -> Option<Value> {
        self.job.lock().unwrap().as_ref().map(|job| json!({"event": "progress", "id": job.id, "fraction": job.progress.fraction(), "kind": job.label}))
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
                        &["csv", "tsv", "psv", "json", "jsonl", "ndjson", "txt", "log"]
                    )
                    .add_filter(language.text("Все файлы", "All files"), &["*"])
                    .pick_file()
                    .map(|p| p.to_string_lossy().into_owned())
            )),
            "open" => {
                let path = PathBuf::from(request["path"].as_str().context("Выберите файл")?);
                let options = &request["options"];
                let config = OpenOptionsConfig {
                    header: options["header"].as_bool().unwrap_or(true),
                    delimiter: options["delimiter"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(|s| s.as_bytes()[0]),
                    format: match options["format"].as_str() {
                        Some("csv") => Format::Csv,
                        Some("json") => Format::Json,
                        Some("text") => Format::Text,
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
                let types = query::infer_types(&data)?;
                let result = json!({"file": id, "revision": id, "name": path.file_name().unwrap_or_default().to_string_lossy(), "path": path.to_string_lossy(), "headers": data.headers, "generated_headers": data.generated_headers, "types": types, "total": data.rows, "rows": data.rows, "bytes": data.bytes, "description": data.description, "irregular": data.irregular_rows, "elapsed": started.elapsed().as_secs_f64()});
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
            "rows" | "cell" | "export" => {
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
                        Ok(
                            json!({"value": preview(value, 1_000_000), "truncated": value.chars().count() > 1_000_000}),
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
