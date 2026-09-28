//! Streaming import and sparse, record-aware indexing for large tables.
use anyhow::{Context, Result, bail};
use csv::{ByteRecord, Position, Reader, ReaderBuilder, Writer};
use encoding_rs::{UTF_16BE, UTF_16LE, WINDOWS_1251, WINDOWS_1252};
use serde::de::{Deserializer as _, SeqAccess, Visitor};
use serde_json::Value;
use std::{
    collections::HashMap,
    fmt,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use tempfile::{NamedTempFile, TempPath};

pub const STRIDE: u64 = 512;
pub const PAGE_SIZE: u64 = 128;
pub const PREVIEW_CHARS: usize = 1024;
const MAX_COLUMNS: usize = 16_384;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Auto,
    Csv,
    Json,
    Text,
    Structured,
    Hex,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    #[default]
    Auto,
    Utf8,
    Utf16Le,
    Utf16Be,
    Windows1251,
    Windows1252,
}

#[derive(Clone, Debug)]
pub struct OpenOptionsConfig {
    pub format: Format,
    pub delimiter: Option<u8>,
    pub header: bool,
    pub encoding: Encoding,
    pub auto_text: bool,
}

impl Default for OpenOptionsConfig {
    fn default() -> Self {
        Self {
            format: Format::Auto,
            delimiter: None,
            header: true,
            encoding: Encoding::Auto,
            auto_text: true,
        }
    }
}

#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub records: AtomicU64,
    pub cancelled: AtomicBool,
    phase: Mutex<(&'static str, &'static str)>,
}

impl Progress {
    pub fn begin(&self, phase: &'static str, total: u64, unit: &'static str) {
        let mut state = self.phase.lock().unwrap();
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
        self.records.store(0, Ordering::Relaxed);
        *state = (phase, unit);
    }
    pub fn snapshot(&self) -> serde_json::Value {
        let state = self.phase.lock().unwrap();
        let (phase, unit) = *state;
        let total = self.total.load(Ordering::Relaxed);
        serde_json::json!({"phase":phase,"unit":unit,"done":self.done.load(Ordering::Relaxed),
            "total":total,"records":self.records.load(Ordering::Relaxed),
            "fraction":if total == 0 {None} else {Some(self.fraction())}})
    }
    pub fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            bail!("Операция отменена");
        }
        Ok(())
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn fraction(&self) -> f32 {
        (self.done.load(Ordering::Relaxed) as f64
            / self.total.load(Ordering::Relaxed).max(1) as f64)
            .clamp(0.0, 1.0) as f32
    }
}

/// Buffer outside this reader so progress/cancellation is checked per I/O block,
/// rather than for every byte consumed by a JSON deserializer.
pub(crate) struct TrackedReader<'a, R> {
    pub input: R,
    pub progress: &'a Progress,
}
impl<R: Read> Read for TrackedReader<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.progress.check().map_err(std::io::Error::other)?;
        let n = self.input.read(bytes)?;
        self.progress.done.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}
impl<R: Seek> Seek for TrackedReader<'_, R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.input.seek(pos)
    }
}

fn looks_like_json(text: &str) -> bool {
    // Logs such as [2026-09-27 12:00] must remain text. An incomplete but
    // syntactically valid JSON prefix is enough for files larger than the probe.
    if !text.starts_with(['{', '[']) {
        return false;
    }
    match serde_json::Deserializer::from_str(text)
        .into_iter::<Value>()
        .next()
    {
        Some(Ok(_)) => true,
        Some(Err(error)) => error.is_eof(),
        None => false,
    }
}

fn structured_or_readable(
    path: &Path,
    progress: &Progress,
    kind: &mut String,
    parse: impl FnOnce() -> Result<crate::formats::Normalized>,
) -> Result<crate::formats::Normalized> {
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(parse)).unwrap_or_else(|_| {
            Err(anyhow::anyhow!(
                "Parser failed on a damaged or unsupported structure"
            ))
        });
    match result {
        Ok(result) => Ok(result),
        Err(error) => {
            progress.check()?;
            let format = kind.clone();
            *kind = "binary".into();
            progress.begin("parse", std::fs::metadata(path)?.len(), "bytes");
            let mut result = crate::formats::binary_readable(path, progress)?;
            result.warnings.insert(0, format!(
                "Could not fully parse {format}: {error:#}. Showing extracted strings; original bytes are available in Hex mode."
            ));
            Ok(result)
        }
    }
}

pub struct Dataset {
    pub original_path: PathBuf,
    pub path: PathBuf,
    pub headers: Vec<String>,
    pub generated_headers: Vec<GeneratedHeader>,
    pub rows: u64,
    pub bytes: u64,
    pub delimiter: u8,
    pub checkpoints: Vec<Position>,
    pub irregular_rows: u64,
    pub description: String,
    pub kind: String,
    pub source_path: PathBuf,
    pub warnings: Vec<String>,
    pub acquisition: String,
    // Keep Windows read locks and temporary files alive until all workers release the dataset.
    _source: File,
    _temporary: Vec<TempPath>,
}

#[derive(serde::Serialize)]
pub struct GeneratedHeader {
    pub column: usize,
    pub kind: &'static str,
}

/// On Windows, deny writes/deletes while the file is open so offsets stay valid.
pub fn open_read(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1); // FILE_SHARE_READ only
    }
    options.open(path).with_context(|| {
        format!(
            "Не удалось открыть {} для чтения. Возможно, файл ещё записывается",
            path.display()
        )
    })
}

pub fn csv_reader(path: &Path, delimiter: u8) -> Result<Reader<File>> {
    Ok(ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .buffer_capacity(256 * 1024)
        .from_reader(open_read(path)?))
}

impl Dataset {
    pub(crate) fn check_export_path(&self, destination: &Path) -> Result<()> {
        let canonical = std::fs::canonicalize(destination).ok();
        for source in [&self.original_path, &self.path, &self.source_path] {
            let same = destination == source
                || canonical.as_ref().is_some_and(|destination| {
                    std::fs::canonicalize(source).is_ok_and(|source| {
                        #[cfg(windows)]
                        {
                            destination.to_string_lossy().to_lowercase()
                                == source.to_string_lossy().to_lowercase()
                        }
                        #[cfg(not(windows))]
                        {
                            *destination == source
                        }
                    })
                });
            if same {
                bail!("Нельзя перезаписать исходный файл");
            }
        }
        Ok(())
    }

    pub fn open(
        path: &Path,
        options: &OpenOptionsConfig,
        progress: &Arc<Progress>,
    ) -> Result<Arc<Self>> {
        match Self::open_inner(path, options, progress, false) {
            Ok(data) => Ok(data),
            Err(error)
                if options.format == Format::Auto
                    && path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                        ["dat", "edb", "bin"]
                            .iter()
                            .any(|known| e.eq_ignore_ascii_case(known))
                    })
                    && error
                        .downcast_ref::<crate::native::NeedsElevation>()
                        .is_none() =>
            {
                progress.check()?;
                let mut data = Self::open_inner(path, options, progress, true)
                    .with_context(|| format!("Automatic import failed: {error:#}"))?;
                Arc::get_mut(&mut data).expect("new dataset is exclusively owned").warnings.insert(0,
                    format!("Could not fully parse binary file: {error:#}. Showing extracted strings; original bytes are available in Hex mode."));
                Ok(data)
            }
            Err(error) => Err(error),
        }
    }

    fn open_inner(
        path: &Path,
        options: &OpenOptionsConfig,
        progress: &Arc<Progress>,
        force_readable: bool,
    ) -> Result<Arc<Self>> {
        progress.begin("read", 0, "bytes");
        let acquired = crate::native::acquire(path, progress)?;
        let source = open_read(&acquired.path)?;
        let bytes = source.metadata()?.len();
        progress.total.store(bytes, Ordering::Relaxed);
        let mut probe = open_read(&acquired.path)?;
        let mut sample = vec![0; 128 * 1024];
        let len = probe.read(&mut sample)?;
        sample.truncate(len);
        let encoding = match options.encoding {
            _ if options.format == Format::Hex || force_readable => Encoding::Utf8,
            Encoding::Auto if sample.starts_with(&[0xFF, 0xFE]) => Encoding::Utf16Le,
            Encoding::Auto if sample.starts_with(&[0xFE, 0xFF]) => Encoding::Utf16Be,
            Encoding::Auto => Encoding::Utf8,
            value => value,
        };
        let mut temporary = acquired.temporary;
        let mut backing = acquired.path;
        if encoding != Encoding::Utf8 {
            progress.begin("decode", bytes, "bytes");
            let encoding_rs = match encoding {
                Encoding::Utf16Le => UTF_16LE,
                Encoding::Utf16Be => UTF_16BE,
                Encoding::Windows1251 => WINDOWS_1251,
                Encoding::Windows1252 => WINDOWS_1252,
                _ => unreachable!(),
            };
            let temp = transcode(&backing, encoding_rs, progress)?;
            backing = temp.path().to_owned();
            temporary.push(temp.into_temp_path());
            let mut file = open_read(&backing)?;
            sample.resize(128 * 1024, 0);
            let len = file.read(&mut sample)?;
            sample.truncate(len);
        }
        let extension = path
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let guessed_delimiter = detect_delimiter(&sample);
        let source_path = backing.clone();
        progress.begin("parse", std::fs::metadata(&backing)?.len(), "bytes");
        let text_sample = String::from_utf8_lossy(&sample);
        let leading = text_sample.trim_start_matches('\u{feff}').trim_start();
        let mut kind = crate::formats::source_kind(&extension)
            .unwrap_or("table")
            .to_owned();
        let structured = if force_readable {
            kind = "binary".into();
            Some(crate::formats::binary_readable(&backing, progress)?)
        } else if options.format == Format::Hex {
            kind = "hex".into();
            Some(crate::formats::hex(&backing, progress)?)
        } else if options.format != Format::Auto {
            None
        } else if acquired.method == "usn-api"
            || extension == "usn"
            || crate::native::is_usn_journal_path(path)
        {
            kind = "usn".into();
            Some(crate::formats::usn(&backing, progress)?)
        } else if extension == "arn"
            && sample.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1])
        {
            kind = "arn".into();
            Some(crate::formats::arn(&backing, progress)?)
        } else if sample.starts_with(b"ElfFile\0") || extension == "evtx" {
            kind = "evtx".into();
            Some(crate::formats::evtx(&backing, progress)?)
        } else if sample.starts_with(b"regf") {
            kind = "registry".into();
            Some(structured_or_readable(
                &backing,
                progress,
                &mut kind,
                || crate::formats::registry(&backing, progress),
            )?)
        } else if sample.starts_with(b"SQLite format 3\0") {
            kind = "sqlite".into();
            Some(crate::formats::sqlite(&backing, progress)?)
        } else if sample.get(4..8) == Some(&[0xEF, 0xCD, 0xAB, 0x89]) {
            kind = "ese".into();
            Some(structured_or_readable(
                &backing,
                progress,
                &mut kind,
                || crate::formats::ese(&backing, progress),
            )?)
        } else if extension == "xml"
            || leading.starts_with("<?xml")
            || leading.starts_with('<')
                && !["md", "markdown", "html", "htm"].contains(&extension.as_str())
        {
            kind = "xml".into();
            Some(crate::formats::xml(&backing, progress)?)
        } else if crate::formats::is_binary(&sample)
            || std::str::from_utf8(&sample).is_err_and(|error| error.error_len().is_some())
        {
            kind = "binary".into();
            Some(crate::formats::binary_readable(&backing, progress)?)
        } else if options.auto_text
            && kind == "table"
            && let Some(headers) = crate::formats::text_table(&sample)
        {
            Some(crate::formats::normalize_text_table(
                &backing, headers, progress,
            )?)
        } else {
            None
        };
        let format = match options.format {
            Format::Hex => Format::Structured,
            Format::Auto if structured.is_some() => Format::Structured,
            Format::Auto if crate::formats::source_kind(&extension).is_some() => Format::Text,
            Format::Auto if ["json", "jsonl", "ndjson"].contains(&extension.as_str()) => {
                Format::Json
            }
            Format::Auto if looks_like_json(leading) => Format::Json,
            Format::Auto if extension == "txt" && !options.auto_text => Format::Text,
            Format::Auto
                if ["csv", "tsv", "psv"].contains(&extension.as_str())
                    || options.delimiter.is_some()
                    || guessed_delimiter.is_some() =>
            {
                Format::Csv
            }
            Format::Auto => Format::Text,
            other => other,
        };
        let mut headers = Vec::new();
        let mut generated_headers = Vec::new();
        let mut warnings = Vec::new();
        let delimiter;
        let has_header;
        match format {
            Format::Structured => {
                let result = structured.context("No structured importer")?;
                backing = result.file.path().to_owned();
                temporary.push(result.file.into_temp_path());
                headers = result.headers;
                warnings = result.warnings;
                delimiter = b',';
                has_header = false;
            }
            Format::Json => {
                kind = if ["jsonl", "ndjson"].contains(&extension.as_str()) {
                    "jsonl"
                } else {
                    "json"
                }
                .into();
                let (temp, names) = normalize_json(&backing, kind == "jsonl", progress)?;
                backing = temp.path().to_owned();
                temporary.push(temp.into_temp_path());
                headers = names;
                delimiter = b',';
                has_header = false;
            }
            Format::Text => {
                if kind == "table" {
                    kind = "text".into();
                }
                let mut temp = NamedTempFile::new()?;
                {
                    let mut writer = Writer::from_writer(temp.as_file_mut());
                    let mut input = BufReader::with_capacity(256 * 1024, open_read(&backing)?);
                    let mut line = String::new();
                    let mut first_line = true;
                    progress.done.store(0, Ordering::Relaxed);
                    progress
                        .total
                        .store(std::fs::metadata(&backing)?.len(), Ordering::Relaxed);
                    loop {
                        progress.check()?;
                        line.clear();
                        let consumed = input.read_line(&mut line).context(
                            "Текст не в UTF-8. Выберите кодировку в параметрах открытия",
                        )?;
                        if consumed == 0 {
                            break;
                        }
                        progress.done.fetch_add(consumed as u64, Ordering::Relaxed);
                        if line.ends_with('\n') {
                            line.pop();
                            if line.ends_with('\r') {
                                line.pop();
                            }
                        }
                        writer.write_record([if first_line {
                            line.strip_prefix('\u{feff}').unwrap_or(&line)
                        } else {
                            &line
                        }])?;
                        progress.records.fetch_add(1, Ordering::Relaxed);
                        first_line = false;
                    }
                    writer.flush()?;
                }
                backing = temp.path().to_owned();
                temporary.push(temp.into_temp_path());
                headers.push("Текст".into());
                generated_headers.push(GeneratedHeader {
                    column: 0,
                    kind: "text",
                });
                delimiter = b',';
                has_header = false;
            }
            _ => {
                delimiter = options.delimiter.unwrap_or(if extension == "tsv" {
                    b'\t'
                } else {
                    guessed_delimiter.unwrap_or(b',')
                });
                has_header = options.header;
            }
        }
        let mut reader = ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(false)
            .flexible(true)
            .buffer_capacity(256 * 1024)
            .from_reader(ValidatedCsv::new(open_read(&backing)?, delimiter));
        let mut record = ByteRecord::new();
        if has_header && reader.read_byte_record(&mut record)? {
            for field in &record {
                headers.push(
                    std::str::from_utf8(field)
                        .context("Заголовок не в UTF-8. Выберите кодировку")?
                        .to_owned(),
                );
            }
        }
        if headers.len() > MAX_COLUMNS {
            bail!("Более {MAX_COLUMNS} столбцов. Проверьте разделитель");
        }
        let expected = headers.len();
        let mut checkpoints = Vec::new();
        let mut rows = 0u64;
        let mut irregular_rows = 0;
        let mut widest = expected;
        progress.begin("index", std::fs::metadata(&backing)?.len(), "bytes");
        if std::fs::metadata(&backing)?.len() >= 32 * 1024 * 1024 {
            let indexed = parallel_index(
                &backing,
                reader.position().byte(),
                delimiter,
                expected,
                format == Format::Csv,
                progress,
            )?;
            checkpoints = indexed.checkpoints;
            rows = indexed.rows;
            irregular_rows = indexed.irregular;
            widest = widest.max(indexed.widest);
        } else {
            loop {
                let mut position = reader.position().clone();
                if !reader
                    .read_byte_record(&mut record)
                    .with_context(|| format!("Ошибка CSV около строки {}", rows + 1))?
                {
                    break;
                }
                if record.len() > MAX_COLUMNS {
                    bail!("Более {MAX_COLUMNS} столбцов. Проверьте разделитель файла");
                }
                std::str::from_utf8(record.as_slice()).with_context(|| format!("Строка {}: данные не в UTF-8. Выберите Windows-1251, Windows-1252 или UTF-16 в параметрах открытия", rows + 1))?;
                if rows.is_multiple_of(STRIDE) {
                    progress.check()?;
                    position.set_record(rows);
                    checkpoints.push(position);
                    progress.records.store(rows, Ordering::Relaxed);
                    progress
                        .done
                        .store(reader.position().byte(), Ordering::Relaxed);
                }
                if expected > 0 && record.len() != expected && format == Format::Csv {
                    irregular_rows += 1;
                }
                widest = widest.max(record.len());
                rows += 1;
            }
        }
        while headers.len() < widest {
            generated_headers.push(GeneratedHeader {
                column: headers.len(),
                kind: "column",
            });
            headers.push(format!("Столбец {}", headers.len() + 1));
        }
        for (column, name) in headers.iter().enumerate() {
            if name.trim().is_empty() {
                generated_headers.push(GeneratedHeader {
                    column,
                    kind: "column",
                });
            }
        }
        make_headers_unique(&mut headers);
        progress.records.store(rows, Ordering::Relaxed);
        progress
            .done
            .store(progress.total.load(Ordering::Relaxed), Ordering::Relaxed);
        progress.check()?;
        Ok(Arc::new(Self {
            original_path: path.to_owned(),
            path: backing,
            headers,
            generated_headers,
            rows,
            bytes,
            delimiter,
            checkpoints,
            irregular_rows,
            description: format!(
                "{} · {}",
                match format {
                    Format::Json if kind == "jsonl" => "JSONL",
                    Format::Json => "JSON",
                    Format::Text if kind == "markdown" => "Markdown",
                    Format::Text if kind != "text" => "Source",
                    Format::Text => "TXT",
                    Format::Structured => match kind.as_str() {
                        "evtx" => "EVTX",
                        "xml" => "XML",
                        "arn" => "Autoruns",
                        "registry" => "Registry",
                        "sqlite" => "SQLite",
                        "ese" => "ESE / EDB",
                        "usn" => "USN Journal",
                        "hex" => "Hex",
                        "binary" => "Binary strings",
                        _ => "TXT table",
                    },
                    _ => "CSV",
                },
                match encoding {
                    Encoding::Windows1251 => "Windows-1251",
                    Encoding::Windows1252 => "Windows-1252",
                    Encoding::Utf16Le => "UTF-16 LE",
                    Encoding::Utf16Be => "UTF-16 BE",
                    _ => "UTF-8",
                }
            ),
            kind,
            source_path,
            warnings,
            acquisition: acquired.method.into(),
            _source: source,
            _temporary: temporary,
        }))
    }

    pub fn reader_at(&self, row: u64) -> Result<Reader<File>> {
        if row >= self.rows {
            bail!("Строка за пределами таблицы");
        }
        let block = self
            .checkpoints
            .partition_point(|p| p.record() <= row)
            .saturating_sub(1);
        let mut reader = csv_reader(&self.path, self.delimiter)?;
        reader.seek(self.checkpoints[block].clone())?;
        let mut skip = ByteRecord::new();
        for _ in self.checkpoints[block].record()..row {
            reader.read_byte_record(&mut skip)?;
        }
        Ok(reader)
    }

    pub fn record(&self, row: u64) -> Result<ByteRecord> {
        let mut reader = self.reader_at(row)?;
        let mut record = ByteRecord::new();
        if !reader.read_byte_record(&mut record)? {
            bail!("Файл изменился или строка недоступна");
        }
        Ok(record)
    }

    pub fn page(&self, start: u64) -> Result<Vec<Vec<String>>> {
        let mut reader = self.reader_at(start)?;
        let mut record = ByteRecord::new();
        let mut rows = Vec::new();
        for _ in start..(start + PAGE_SIZE).min(self.rows) {
            if !reader.read_byte_record(&mut record)? {
                break;
            }
            rows.push(
                record
                    .iter()
                    .map(|f| preview(std::str::from_utf8(f).unwrap_or("�"), PREVIEW_CHARS))
                    .collect(),
            );
        }
        Ok(rows)
    }
}

pub fn preview(value: &str, limit: usize) -> String {
    match value.char_indices().nth(limit) {
        Some((boundary, _)) => format!("{}…", &value[..boundary]),
        None => value.to_owned(),
    }
}

fn transcode(
    path: &Path,
    encoding: &'static encoding_rs::Encoding,
    progress: &Progress,
) -> Result<NamedTempFile> {
    let mut file = open_read(path)?;
    let mut temp =
        NamedTempFile::new().context("Не удалось создать временный файл для декодирования")?;
    let mut decoder = encoding.new_decoder_with_bom_removal();
    let mut input = vec![0; 256 * 1024];
    let mut output = vec![0; 768 * 1024];
    loop {
        progress.check()?;
        let n = file.read(&mut input)?;
        progress.done.fetch_add(n as u64, Ordering::Relaxed);
        let mut consumed = 0;
        loop {
            let (result, read, written, errors) =
                decoder.decode_to_utf8(&input[consumed..n], &mut output, n == 0);
            if errors {
                bail!(
                    "Повреждённая последовательность в кодировке {}. Проверьте выбранную кодировку",
                    encoding.name()
                );
            }
            consumed += read;
            temp.write_all(&output[..written])?;
            if result == encoding_rs::CoderResult::InputEmpty {
                break;
            }
        }
        if n == 0 {
            break;
        }
    }
    temp.flush()?;
    Ok(temp)
}

/// The csv crate intentionally accepts malformed quotes. Validate the structure
/// while indexing so a truncated quoted field cannot silently merge many rows.
struct ValidatedCsv<R: Read> {
    file: R,
    delimiter: u8,
    state: QuoteState,
    bytes: u64,
}
#[derive(Clone, Copy, PartialEq)]
enum QuoteState {
    Start,
    Unquoted,
    Quoted,
    AfterQuote,
}

struct IndexedChunk {
    checkpoints: Vec<Position>,
    rows: u64,
    irregular: u64,
    widest: usize,
}

/// Compute a chunk's transition for each possible initial quote state. Prefix
/// composition then finds true record boundaries, including multiline fields,
/// escaped quotes and literal quotes in unquoted fields. No guessed line splits.
fn transition(
    bytes: &[u8],
    mut state: QuoteState,
    delimiter: u8,
) -> Option<(QuoteState, Option<usize>)> {
    let mut i = 0;
    let mut boundary = None;
    while i < bytes.len() {
        match state {
            QuoteState::Start => match bytes[i] {
                b'"' => {
                    state = QuoteState::Quoted;
                    i += 1;
                }
                b'\n' | b'\r' => {
                    i += 1;
                    boundary.get_or_insert(i);
                }
                byte if byte == delimiter => i += 1,
                _ => state = QuoteState::Unquoted,
            },
            QuoteState::Unquoted => {
                if let Some(next) = memchr::memchr3(delimiter, b'\n', b'\r', &bytes[i..]) {
                    i += next;
                    if bytes[i] != delimiter {
                        boundary.get_or_insert(i + 1);
                    }
                    i += 1;
                    state = QuoteState::Start;
                } else {
                    break;
                }
            }
            QuoteState::Quoted => {
                if let Some(next) = memchr::memchr(b'"', &bytes[i..]) {
                    i += next + 1;
                    state = QuoteState::AfterQuote;
                } else {
                    break;
                }
            }
            QuoteState::AfterQuote => {
                match bytes[i] {
                    b'"' => state = QuoteState::Quoted,
                    b'\n' | b'\r' => {
                        state = QuoteState::Start;
                        boundary.get_or_insert(i + 1);
                    }
                    byte if byte == delimiter => state = QuoteState::Start,
                    _ => return None,
                }
                i += 1;
            }
        }
    }
    Some((state, boundary))
}

fn parallel_index(
    path: &Path,
    mut start: u64,
    delimiter: u8,
    expected: usize,
    csv_format: bool,
    progress: &Progress,
) -> Result<IndexedChunk> {
    const CHUNK: u64 = 8 * 1024 * 1024;
    let bytes = std::fs::metadata(path)?.len();
    // Strip a BOM before computing quote-state transitions, as the CSV reader does.
    if start == 0 {
        let mut bom = [0; 3];
        if open_read(path)?.read(&mut bom)? == 3 && bom == [0xef, 0xbb, 0xbf] {
            start = 3;
        }
    }
    let chunks = (bytes - start).div_ceil(CHUNK) as usize;
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 16)
        .min(chunks.max(1));
    progress.total.store((bytes - start) * 2, Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let summaries = std::thread::scope(|scope| -> Result<Vec<_>> {
        let mut handles = Vec::new();
        for worker in 0..workers {
            handles.push(scope.spawn(move || -> Result<Vec<_>> {
                let mut file = open_read(path)?;
                let mut buffer = vec![0; CHUNK as usize];
                let mut results = Vec::new();
                for chunk in (worker..chunks).step_by(workers) {
                    progress.check()?;
                    let offset = start + chunk as u64 * CHUNK;
                    let length = (bytes - offset).min(CHUNK) as usize;
                    file.seek(SeekFrom::Start(offset))?;
                    file.read_exact(&mut buffer[..length])?;
                    let states = [
                        QuoteState::Start,
                        QuoteState::Unquoted,
                        QuoteState::Quoted,
                        QuoteState::AfterQuote,
                    ]
                    .map(|state| transition(&buffer[..length], state, delimiter));
                    results.push((chunk, states));
                    progress.done.fetch_add(length as u64, Ordering::Relaxed);
                }
                Ok(results)
            }));
        }
        let mut results = Vec::new();
        let mut error = None;
        for handle in handles {
            match handle
                .join()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Сбой индексации CSV")))
            {
                Ok(mut chunk) => results.append(&mut chunk),
                Err(err) => {
                    progress.cancel();
                    if error
                        .as_ref()
                        .is_none_or(|old: &anyhow::Error| old.to_string() == "Операция отменена")
                    {
                        error = Some(err);
                    }
                }
            }
        }
        if let Some(error) = error {
            return Err(error);
        }
        results.sort_unstable_by_key(|(chunk, _)| *chunk);
        Ok(results)
    })?;
    let mut state = QuoteState::Start;
    let mut boundaries = vec![start];
    for (chunk, states) in summaries {
        let (end, first) =
            states[state as usize].context("Некорректные символы после кавычки CSV")?;
        if let Some(boundary) = first {
            let offset = start + chunk as u64 * CHUNK + boundary as u64;
            if offset > *boundaries.last().unwrap() && offset < bytes {
                boundaries.push(offset);
            }
        }
        state = end;
    }
    if state == QuoteState::Quoted {
        bail!("Незакрытые кавычки CSV в конце файла");
    }
    boundaries.push(bytes);
    let shards = std::thread::scope(|scope| -> Result<Vec<_>> {
        let mut handles = Vec::new();
        for worker in 0..workers {
            let boundaries = &boundaries;
            handles.push(scope.spawn(move || -> Result<Vec<_>> {
                let mut results = Vec::new();
                for chunk in (worker..boundaries.len() - 1).step_by(workers) {
                    progress.check()?;
                    let begin = boundaries[chunk];
                    let length = boundaries[chunk + 1] - begin;
                    let mut file = open_read(path)?;
                    file.seek(SeekFrom::Start(begin))?;
                    let mut reader = ReaderBuilder::new()
                        .delimiter(delimiter)
                        .has_headers(false)
                        .flexible(true)
                        .buffer_capacity(256 * 1024)
                        // An embedded BOM is data, not a new file BOM. Consume a
                        // synthetic record before this slice, then subtract it
                        // from every byte offset stored in the index.
                        .from_reader(std::io::Cursor::new(b"_\n").chain(file.take(length)));
                    let mut record = ByteRecord::new();
                    reader.read_byte_record(&mut record)?;
                    let mut indexed = IndexedChunk {
                        checkpoints: Vec::new(),
                        rows: 0,
                        irregular: 0,
                        widest: expected,
                    };
                    let mut reported = 0;
                    loop {
                        let mut position = reader.position().clone();
                        if !reader.read_byte_record(&mut record)? {
                            break;
                        }
                        if record.len() > MAX_COLUMNS {
                            bail!("Более {MAX_COLUMNS} столбцов. Проверьте разделитель файла");
                        }
                        std::str::from_utf8(record.as_slice())
                            .context("Данные не в UTF-8. Выберите кодировку в настройках")?;
                        if indexed.rows.is_multiple_of(STRIDE) {
                            progress.check()?;
                            position
                                .set_byte(begin + position.byte() - 2)
                                .set_record(indexed.rows);
                            indexed.checkpoints.push(position);
                            progress.done.fetch_add(
                                reader.position().byte() - 2 - reported,
                                Ordering::Relaxed,
                            );
                            reported = reader.position().byte() - 2;
                        }
                        if expected > 0 && record.len() != expected && csv_format {
                            indexed.irregular += 1;
                        }
                        indexed.widest = indexed.widest.max(record.len());
                        indexed.rows += 1;
                    }
                    progress
                        .done
                        .fetch_add(length.saturating_sub(reported), Ordering::Relaxed);
                    results.push((chunk, indexed));
                }
                Ok(results)
            }));
        }
        let mut results = Vec::new();
        let mut error = None;
        for handle in handles {
            match handle
                .join()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Сбой индексации CSV")))
            {
                Ok(mut chunks) => results.append(&mut chunks),
                Err(err) => {
                    progress.cancel();
                    if error
                        .as_ref()
                        .is_none_or(|old: &anyhow::Error| old.to_string() == "Операция отменена")
                    {
                        error = Some(err);
                    }
                }
            }
        }
        if let Some(error) = error {
            return Err(error);
        }
        results.sort_unstable_by_key(|(chunk, _)| *chunk);
        Ok(results)
    })?;
    let mut result = IndexedChunk {
        checkpoints: Vec::new(),
        rows: 0,
        irregular: 0,
        widest: expected,
    };
    for (_, mut shard) in shards {
        for position in &mut shard.checkpoints {
            position.set_record(result.rows + position.record());
        }
        result.checkpoints.append(&mut shard.checkpoints);
        result.rows += shard.rows;
        result.irregular += shard.irregular;
        result.widest = result.widest.max(shard.widest);
    }
    Ok(result)
}
impl<R: Read> ValidatedCsv<R> {
    fn new(file: R, delimiter: u8) -> Self {
        Self {
            file,
            delimiter,
            state: QuoteState::Start,
            bytes: 0,
        }
    }
}
impl<R: Read> Read for ValidatedCsv<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let n = self.file.read(buffer)?;
        if n == 0 && self.state == QuoteState::Quoted {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Незакрытые кавычки CSV в конце файла",
            ));
        }
        let mut i = if self.bytes == 0 && buffer[..n].starts_with(&[0xEF, 0xBB, 0xBF]) {
            3
        } else {
            0
        };
        while i < n {
            match self.state {
                QuoteState::Start => match buffer[i] {
                    b'"' => {
                        self.state = QuoteState::Quoted;
                        i += 1;
                    }
                    b'\n' | b'\r' => i += 1,
                    byte if byte == self.delimiter => i += 1,
                    _ => self.state = QuoteState::Unquoted,
                },
                QuoteState::Unquoted => {
                    if let Some(next) = memchr::memchr3(self.delimiter, b'\n', b'\r', &buffer[i..n])
                    {
                        i += next + 1;
                        self.state = QuoteState::Start;
                    } else {
                        i = n;
                    }
                }
                QuoteState::Quoted => {
                    if let Some(next) = memchr::memchr(b'"', &buffer[i..n]) {
                        i += next + 1;
                        self.state = QuoteState::AfterQuote;
                    } else {
                        i = n;
                    }
                }
                QuoteState::AfterQuote => {
                    match buffer[i] {
                        b'"' => self.state = QuoteState::Quoted,
                        b'\n' | b'\r' => self.state = QuoteState::Start,
                        byte if byte == self.delimiter => self.state = QuoteState::Start,
                        _ => {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!(
                                    "Некорректные символы после закрывающей кавычки CSV, байт {}",
                                    self.bytes + i as u64 + 1
                                ),
                            ));
                        }
                    }
                    i += 1;
                }
            }
        }
        self.bytes += n as u64;
        Ok(n)
    }
}

fn make_headers_unique(headers: &mut [String]) {
    let mut used = std::collections::HashSet::new();
    for (index, name) in headers.iter_mut().enumerate() {
        if name.is_empty() {
            *name = format!("Столбец {}", index + 1);
        }
        let base = name.clone();
        let mut suffix = 2;
        while !used.insert(name.clone()) {
            *name = format!("{base} ({suffix})");
            suffix += 1;
        }
    }
}

pub fn detect_delimiter(sample: &[u8]) -> Option<u8> {
    [b',', b'\t', b';', b'|']
        .into_iter()
        .filter_map(|delimiter| {
            let mut reader = ReaderBuilder::new()
                .has_headers(false)
                .flexible(true)
                .delimiter(delimiter)
                .from_reader(sample);
            let mut counts = HashMap::<usize, usize>::new();
            let mut total = 0;
            for row in reader.byte_records().take(32).flatten() {
                *counts.entry(row.len()).or_default() += 1;
                total += 1;
            }
            let (width, consistent) = counts
                .into_iter()
                .filter(|(width, _)| *width > 1)
                .max_by_key(|(_, n)| *n)?;
            if consistent * 100 < total * 80 {
                return None;
            }
            Some((delimiter, consistent * 1000 + width.min(999)))
        })
        .max_by_key(|(_, score)| *score)
        .map(|(delimiter, _)| delimiter)
}

fn flatten(value: Value, path: &str, output: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, value) in map {
                let key = key.replace('~', "~0").replace('/', "~1");
                flatten(value, &format!("{path}/{key}"), output);
            }
        }
        Value::String(text) => {
            output.push((if path.is_empty() { "value" } else { path }.into(), text))
        }
        other => output.push((
            if path.is_empty() { "value" } else { path }.into(),
            other.to_string(),
        )),
    }
}

struct JsonRows<'a, F>(&'a mut F);
impl<'de, F: FnMut(Value) -> Result<()>> Visitor<'de> for JsonRows<'_, F> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("JSON array")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
        while let Some(value) = seq.next_element::<Value>()? {
            (self.0)(value).map_err(serde::de::Error::custom)?;
        }
        Ok(())
    }
}

fn normalize_json(
    path: &Path,
    json_lines: bool,
    progress: &Progress,
) -> Result<(NamedTempFile, Vec<String>)> {
    let mut temp = NamedTempFile::new()?;
    let mut names = Vec::new();
    {
        let mut columns = HashMap::<String, usize>::new();
        let mut writer = csv::WriterBuilder::new()
            .flexible(true)
            .from_writer(temp.as_file_mut());
        let mut input = BufReader::with_capacity(
            256 * 1024,
            TrackedReader {
                input: open_read(path)?,
                progress,
            },
        );
        let mut prefix = [0u8; 3];
        let n = input.read(&mut prefix)?;
        input.seek(SeekFrom::Start(if n == 3 && prefix == [0xEF, 0xBB, 0xBF] {
            3
        } else {
            0
        }))?;
        let first = loop {
            let buffer = input.fill_buf()?;
            if buffer.is_empty() {
                break None;
            }
            if let Some(index) = buffer.iter().position(|b| !b.is_ascii_whitespace()) {
                let byte = buffer[index];
                input.consume(index);
                break Some(byte);
            }
            let n = buffer.len();
            input.consume(n);
        };
        let mut append = |value| -> Result<()> {
            progress.check()?;
            let mut fields = Vec::new();
            flatten(value, "", &mut fields);
            let mut record = vec![String::new(); names.len()];
            for (name, text) in fields {
                let index = *columns.entry(name.clone()).or_insert_with(|| {
                    let index = names.len();
                    names.push(name);
                    index
                });
                if index >= MAX_COLUMNS {
                    bail!("JSON содержит более {MAX_COLUMNS} столбцов");
                }
                if record.len() <= index {
                    record.resize(index + 1, String::new());
                }
                record[index] = text;
            }
            writer.write_record(record)?;
            progress.records.fetch_add(1, Ordering::Relaxed);
            Ok(())
        };
        if first == Some(b'[') && !json_lines {
            let mut deserializer = serde_json::Deserializer::from_reader(input);
            deserializer
                .deserialize_seq(JsonRows(&mut append))
                .context("Ошибка массива JSON")?;
            deserializer
                .end()
                .context("Лишние данные после массива JSON")?;
        } else if first.is_some() {
            for value in serde_json::Deserializer::from_reader(input).into_iter::<Value>() {
                append(value.context("Ошибка JSON / JSON Lines")?)?;
            }
        }
        writer.flush()?;
    }
    Ok((temp, names))
}
