//! Parallel, record-aligned scans. Every occurrence is stored on disk, never capped.
use crate::data::{Dataset, Progress, STRIDE};
use anyhow::{Context, Result, bail};
use csv::ByteRecord;
use regex::{Regex, RegexBuilder};
use roaring::RoaringTreemap;
use std::{
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Table,
    Column(usize),
    Cell { row: u64, column: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindSpec {
    pub text: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub scope: Scope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterOp {
    Contains,
    NotContains,
    Equals,
    NotEquals,
    StartsWith,
    Regex,
    Empty,
    NotEmpty,
}

impl FilterOp {
    pub const ALL: [Self; 8] = [
        Self::Contains,
        Self::NotContains,
        Self::Equals,
        Self::NotEquals,
        Self::StartsWith,
        Self::Regex,
        Self::Empty,
        Self::NotEmpty,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Contains => "Содержит",
            Self::NotContains => "Не содержит",
            Self::Equals => "Равно",
            Self::NotEquals => "Не равно",
            Self::StartsWith => "Начинается с",
            Self::Regex => "Regex",
            Self::Empty => "Пусто",
            Self::NotEmpty => "Не пусто",
        }
    }
}

#[derive(Clone, Debug)]
pub struct FilterRule {
    pub column: usize,
    pub op: FilterOp,
    pub text: String,
    pub case_sensitive: bool,
}

#[derive(Clone, Default)]
pub struct ScanRequest {
    pub find: Option<FindSpec>,
    pub filters: Vec<FilterRule>,
    pub restrict: Option<Arc<RoaringTreemap>>,
}

pub fn compile_pattern(spec: &FindSpec) -> Result<Regex> {
    if spec.text.is_empty() {
        bail!("Введите текст для поиска");
    }
    let pattern = if spec.regex {
        spec.text.clone()
    } else {
        regex::escape(&spec.text)
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!spec.case_sensitive)
        .size_limit(16 * 1024 * 1024)
        .build()
        .context("Некорректное регулярное выражение")
}

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    #[default]
    Auto,
    Text,
    Regex,
}

/// Compile once per query; clones give scan workers independent regex scratch space.
#[derive(Clone)]
pub struct TableSearch {
    pattern: Regex,
    extensions: Vec<String>,
}
impl TableSearch {
    pub fn new(text: &str, mode: SearchMode, case_sensitive: bool) -> Result<Self> {
        let mut pattern = text;
        let mut extensions = Vec::new();
        let mut is_regex = matches!(mode, SearchMode::Regex);
        if !matches!(mode, SearchMode::Text) {
            pattern = pattern.trim();
            if pattern
                .get(..4)
                .is_some_and(|s| s.eq_ignore_ascii_case("ext:"))
            {
                let end = pattern.find(char::is_whitespace).unwrap_or(pattern.len());
                for extension in pattern[4..end].split(';') {
                    let extension = extension.trim_start_matches('.');
                    if extension.is_empty() || extension.contains(['/', '\\', ':', '*', '?']) {
                        bail!("Некорректный список ext: (пример: ext:exe;jar;zip)");
                    }
                    extensions.push(extension.to_ascii_lowercase());
                }
                pattern = pattern[end..].trim_start();
            }
            if pattern
                .get(..6)
                .is_some_and(|s| s.eq_ignore_ascii_case("regex:"))
            {
                pattern = &pattern[6..];
                is_regex = true;
            } else if pattern.starts_with("(?") {
                is_regex = true;
            }
        }
        let pattern = if is_regex {
            pattern.to_owned()
        } else {
            regex::escape(pattern)
        };
        Ok(Self {
            pattern: RegexBuilder::new(&pattern)
                .case_insensitive(!case_sensitive)
                .size_limit(32 * 1024 * 1024)
                .dfa_size_limit(16 * 1024 * 1024)
                .build()
                .context("Некорректное регулярное выражение")?,
            extensions,
        })
    }

    pub fn matches_value(&self, value: &str) -> bool {
        if !self.extensions.is_empty() {
            let path = value.trim().trim_matches('"');
            let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
            // NTFS alternate streams inherit the base file's extension.
            let name = name.split(':').next().unwrap_or(name);
            let Some((_, extension)) = name.rsplit_once('.') else {
                return false;
            };
            if !self
                .extensions
                .iter()
                .any(|e| extension.eq_ignore_ascii_case(e))
            {
                return false;
            }
        }
        self.pattern.is_match(value)
    }

    pub fn matches(&self, record: &ByteRecord) -> bool {
        record
            .iter()
            .any(|v| self.matches_value(std::str::from_utf8(v).unwrap_or("")))
    }
}

#[derive(Clone)]
pub(crate) struct CompiledFilter {
    column: usize,
    op: FilterOp,
    regex: Regex,
}
impl CompiledFilter {
    pub(crate) fn new(rule: &FilterRule) -> Result<Self> {
        let escaped = regex::escape(&rule.text);
        let pattern = match rule.op {
            FilterOp::Regex => rule.text.clone(),
            FilterOp::Equals | FilterOp::NotEquals => format!("\\A{escaped}\\z"),
            FilterOp::StartsWith => format!("\\A{escaped}"),
            _ => escaped,
        };
        Ok(Self {
            column: rule.column,
            op: rule.op,
            regex: RegexBuilder::new(&pattern)
                .case_insensitive(!rule.case_sensitive)
                .size_limit(16 * 1024 * 1024)
                .build()
                .context("Ошибка regex в фильтре")?,
        })
    }
    pub(crate) fn matches(&self, record: &ByteRecord) -> bool {
        let value = std::str::from_utf8(record.get(self.column).unwrap_or(b"")).unwrap_or("");
        match self.op {
            FilterOp::Empty => value.is_empty(),
            FilterOp::NotEmpty => !value.is_empty(),
            FilterOp::NotContains | FilterOp::NotEquals => !self.regex.is_match(value),
            _ => self.regex.is_match(value),
        }
    }
}

/// Fixed 32-byte records keep result lookup O(1), including beyond 4 GiB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub row: u64,
    pub column: u64,
    pub start: u64,
    pub end: u64,
}
impl Hit {
    fn write(self, writer: &mut impl Write) -> Result<()> {
        for word in [self.row, self.column, self.start, self.end] {
            writer.write_all(&word.to_le_bytes())?;
        }
        Ok(())
    }
    fn read(reader: &mut impl Read) -> Result<Self> {
        let mut data = [0; 32];
        reader.read_exact(&mut data)?;
        let word = |start| u64::from_le_bytes(data[start..start + 8].try_into().unwrap());
        Ok(Self {
            row: word(0),
            column: word(8),
            start: word(16),
            end: word(24),
        })
    }
}

struct Shard {
    file: NamedTempFile,
    count: u64,
    rows: RoaringTreemap,
}

pub struct SearchOutput {
    shards: Vec<Shard>,
    pub matched_rows: Arc<RoaringTreemap>,
    pub hits: u64,
    pub elapsed: Duration,
}

impl SearchOutput {
    pub fn hit_page(&self, start: u64, count: u64) -> Result<Vec<Hit>> {
        let mut skip = start;
        let mut hits = Vec::new();
        for shard in &self.shards {
            if skip >= shard.count {
                skip -= shard.count;
                continue;
            }
            let mut file = shard.file.reopen()?;
            file.seek(SeekFrom::Start(
                skip.checked_mul(32)
                    .context("Слишком большой индекс результатов")?,
            ))?;
            for _ in skip..shard.count {
                if hits.len() as u64 == count {
                    return Ok(hits);
                }
                hits.push(Hit::read(&mut file)?);
            }
            skip = 0;
        }
        Ok(hits)
    }
}

pub fn scan(
    dataset: &Dataset,
    request: &ScanRequest,
    progress: &Arc<Progress>,
) -> Result<SearchOutput> {
    let started = Instant::now();
    let regex = request.find.as_ref().map(compile_pattern).transpose()?;
    let filters: Vec<_> = request
        .filters
        .iter()
        .map(CompiledFilter::new)
        .collect::<Result<_>>()?;
    if let Some(find) = &request.find {
        match find.scope {
            Scope::Column(c) | Scope::Cell { column: c, .. } if c >= dataset.headers.len() => {
                bail!("Столбец недоступен")
            }
            Scope::Cell { row, .. } if row >= dataset.rows => bail!("Ячейка недоступна"),
            _ => {}
        }
    }
    let blocks = dataset.rows.div_ceil(STRIDE);
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 8)
        .min(blocks.max(1) as usize);
    let cell_row = request.find.as_ref().and_then(|f| {
        if let Scope::Cell { row, .. } = f.scope {
            Some(row)
        } else {
            None
        }
    });
    let worker_count = if cell_row.is_some() { 1 } else { workers };
    progress.total.store(
        if cell_row.is_some() { 1 } else { dataset.rows },
        Ordering::Relaxed,
    );
    progress.done.store(0, Ordering::Relaxed);
    let shards = std::thread::scope(|scope| -> Result<Vec<Shard>> {
        let mut handles = Vec::new();
        for worker in 0..worker_count {
            let begin = cell_row
                .unwrap_or(worker as u64 * blocks.div_ceil(workers as u64) * STRIDE)
                .min(dataset.rows);
            let end = cell_row
                .map(|r| r + 1)
                .unwrap_or((worker + 1) as u64 * blocks.div_ceil(workers as u64) * STRIDE)
                .min(dataset.rows);
            let regex = regex.clone();
            let filters = filters.clone();
            handles.push(scope.spawn(move || -> Result<Shard> {
                let mut file =
                    NamedTempFile::new().context("Не удалось создать временный файл поиска")?;
                let mut count = 0;
                let mut rows = RoaringTreemap::new();
                if begin >= end {
                    return Ok(Shard { file, count, rows });
                }
                {
                    let mut output = BufWriter::with_capacity(256 * 1024, file.as_file_mut());
                    let mut reader = dataset.reader_at(begin)?;
                    let mut record = ByteRecord::new();
                    for row in begin..end {
                        if (row - begin).is_multiple_of(256) {
                            progress.check()?;
                        }
                        if !reader.read_byte_record(&mut record)? {
                            bail!("Неожиданный конец файла при поиске");
                        }
                        if request
                            .restrict
                            .as_ref()
                            .is_none_or(|mask| mask.contains(row))
                            && filters.iter().all(|f| f.matches(&record))
                        {
                            if let (Some(regex), Some(find)) = (&regex, &request.find) {
                                let range = match find.scope {
                                    Scope::Table => 0..dataset.headers.len(),
                                    Scope::Column(c) | Scope::Cell { column: c, .. } => c..c + 1,
                                };
                                let before = count;
                                for column in range {
                                    let value =
                                        std::str::from_utf8(record.get(column).unwrap_or(b""))
                                            .unwrap_or("");
                                    for m in regex.find_iter(value) {
                                        if count.is_multiple_of(4096) {
                                            progress.check()?;
                                        }
                                        Hit {
                                            row,
                                            column: column as u64,
                                            start: m.start() as u64,
                                            end: m.end() as u64,
                                        }
                                        .write(&mut output)?;
                                        count += 1;
                                    }
                                }
                                if before != count {
                                    rows.insert(row);
                                }
                            } else {
                                rows.insert(row);
                            }
                        }
                        if (row - begin + 1).is_multiple_of(256) {
                            progress.done.fetch_add(256, Ordering::Relaxed);
                        }
                    }
                    progress
                        .done
                        .fetch_add((end - begin) % 256, Ordering::Relaxed);
                    output.flush()?;
                }
                Ok(Shard { file, count, rows })
            }));
        }
        // Join every worker before returning, preserving errors and cleaning all temp files.
        let mut shards = Vec::new();
        let mut error = None;
        for handle in handles {
            match handle
                .join()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Сбой потока поиска")))
            {
                Ok(shard) => shards.push(shard),
                Err(err) => {
                    progress.cancel();
                    if error.is_none() {
                        error = Some(err);
                    }
                }
            }
        }
        if let Some(err) = error {
            return Err(err);
        }
        Ok(shards)
    })?;
    progress.check()?;
    let mut matched_rows = RoaringTreemap::new();
    let mut hits = 0;
    for shard in &shards {
        matched_rows |= &shard.rows;
        hits += shard.count;
    }
    // Drop duplicate per-worker bitmaps once the merged index exists.
    let shards = shards
        .into_iter()
        .map(|mut s| {
            s.rows = RoaringTreemap::new();
            s
        })
        .collect();
    Ok(SearchOutput {
        shards,
        matched_rows: Arc::new(matched_rows),
        hits,
        elapsed: started.elapsed(),
    })
}

pub fn export_csv(
    dataset: &Dataset,
    mask: Option<&RoaringTreemap>,
    path: &std::path::Path,
    progress: &Progress,
) -> Result<u64> {
    // Write atomically beside the destination. The input remains read-only.
    dataset.check_export_path(path)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let mut temp = NamedTempFile::new_in(parent)?;
    let mut written = 0;
    {
        let mut writer = csv::Writer::from_writer(temp.as_file_mut());
        writer.write_record(&dataset.headers)?;
        progress.total.store(dataset.rows, Ordering::Relaxed);
        if dataset.rows > 0 {
            let mut reader = dataset.reader_at(0)?;
            let mut record = ByteRecord::new();
            for row in 0..dataset.rows {
                if row.is_multiple_of(256) {
                    progress.check()?;
                    progress.done.store(row, Ordering::Relaxed);
                }
                if !reader.read_byte_record(&mut record)? {
                    bail!("Неожиданный конец файла");
                }
                if mask.is_none_or(|m| m.contains(row)) {
                    while record.len() < dataset.headers.len() {
                        record.push_field(b"");
                    }
                    writer.write_byte_record(&record)?;
                    written += 1;
                }
            }
        }
        writer.flush()?;
    }
    progress.check()?;
    temp.persist(path).context("Не удалось сохранить CSV")?;
    Ok(written)
}
