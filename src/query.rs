//! Whole-dataset views. The webview receives only a bounded, visible rectangle.
//! Sorted/filtered views retain row numbers and byte offsets, not entire records.
use crate::{
    data::{Dataset, Progress, preview},
    search::{CompiledFilter, FilterOp, FilterRule, FindSpec, Scope, compile_pattern},
};
use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, NaiveDate, NaiveDateTime};
use csv::{ByteRecord, Position};
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, path::Path, sync::atomic::Ordering as AtomicOrdering};
use tempfile::NamedTempFile;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ColumnType {
    #[default]
    Text,
    Number,
    Date,
    Timestamp,
}

pub fn number(value: &str) -> Option<f64> {
    let clean: String = value
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '\u{a0}' | '\u{202f}'))
        .collect();
    let clean = if clean.contains(',') && !clean.contains('.') {
        clean.replace(',', ".")
    } else {
        clean
    };
    clean.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Day-first for ambiguous local dates; ISO/RFC3339 offsets normalize to UTC.
fn instant<Tz: chrono::TimeZone>(date: DateTime<Tz>) -> i128 {
    i128::from(date.timestamp()) * 1_000_000_000 + i128::from(date.timestamp_subsec_nanos())
}

pub fn date(value: &str) -> Option<i128> {
    let value = value.trim();
    let value = value
        .strip_suffix(" UTC")
        .or_else(|| value.strip_suffix(" GMT"))
        .unwrap_or(value);
    if value.len() < 8 || value.len() > 80 {
        return None;
    }
    if let Ok(date) = DateTime::parse_from_rfc3339(value) {
        return Some(instant(date));
    }
    if let Ok(date) = DateTime::parse_from_rfc2822(value) {
        return Some(instant(date));
    }
    let formats: &[&str] = if value
        .as_bytes()
        .get(4)
        .is_some_and(|c| matches!(c, b'-' | b'/'))
    {
        &[
            "%Y-%m-%d %H:%M:%S%.f%:z",
            "%Y-%m-%d %H:%M:%S%.f %:z",
            "%Y-%m-%d %H:%M:%S%.f%z",
            "%Y-%m-%d %H:%M:%S%.f %z",
            "%Y-%m-%dT%H:%M:%S%.f",
            "%Y-%m-%d %H:%M:%S%.f",
            "%Y-%m-%d %H:%M",
            "%Y/%m/%d %H:%M:%S",
        ]
    } else {
        &[
            "%d.%m.%Y %H:%M:%S%.f",
            "%d/%m/%Y %H:%M:%S%.f",
            "%d-%m-%Y %H:%M:%S%.f",
            "%d.%m.%Y %H:%M",
            "%d/%m/%Y %H:%M",
        ]
    };
    for format in formats {
        if format.contains("%:z") || format.contains("%z") {
            if let Ok(date) = DateTime::parse_from_str(value, format) {
                return Some(instant(date));
            }
        } else if let Ok(date) = NaiveDateTime::parse_from_str(value, format) {
            return Some(instant(date.and_utc()));
        }
    }
    for format in ["%Y-%m-%d", "%Y/%m/%d", "%d.%m.%Y", "%d/%m/%Y", "%d-%m-%Y"] {
        if let Ok(date) = NaiveDate::parse_from_str(value, format) {
            return Some(instant(date.and_hms_opt(0, 0, 0)?.and_utc()));
        }
    }
    None
}

fn timestamp(value: &str) -> Option<i128> {
    if let Some(date) = date(value) {
        return Some(date);
    }
    let value = value.trim();
    let n = value.parse::<i128>().ok()?;
    match value.trim_start_matches('-').len() {
        1..=10 => n.checked_mul(1_000_000_000),
        13 => n.checked_mul(1_000_000),
        16 => n.checked_mul(1_000),
        19 => Some(n),
        _ => None,
    }
}

pub fn infer_types(data: &Dataset) -> Result<Vec<ColumnType>> {
    let mut stats = vec![(0, 0, 0, 0); data.headers.len()];
    // Sample the beginning and the end without walking a multi-gigabyte file twice.
    for (start, count) in [
        (0, data.rows.min(512)),
        (
            data.rows.saturating_sub(128).max(data.rows.min(512)),
            data.rows.saturating_sub(512).min(128),
        ),
    ] {
        if count == 0 {
            continue;
        }
        let mut reader = data.reader_at(start)?;
        let mut record = ByteRecord::new();
        for _ in 0..count {
            if !reader.read_byte_record(&mut record)? {
                break;
            }
            for (column, stats) in stats.iter_mut().enumerate() {
                let text = std::str::from_utf8(record.get(column).unwrap_or_default())
                    .unwrap_or("")
                    .trim();
                if text.is_empty() {
                    continue;
                }
                stats.0 += 1;
                stats.1 += usize::from(number(text).is_some());
                stats.2 += usize::from(date(text).is_some());
                stats.3 += usize::from(timestamp(text).is_some());
            }
        }
    }
    Ok(stats
        .iter()
        .enumerate()
        .map(|(column, &(total, numbers, dates, times))| {
            let header = data.headers[column].to_lowercase();
            if total == 0 {
                ColumnType::Text
            } else if dates == total {
                ColumnType::Date
            } else if times == total
                && ["timestamp", "epoch", "unix"]
                    .iter()
                    .any(|hint| header.contains(hint))
            {
                ColumnType::Timestamp
            } else if numbers == total {
                ColumnType::Number
            } else {
                ColumnType::Text
            }
        })
        .collect())
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Query {
    pub text: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub filters: Vec<Filter>,
    pub sort: Option<Sort>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Filter {
    pub column: usize,
    pub op: String,
    #[serde(default)]
    pub text: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Sort {
    pub column: usize,
    #[serde(default)]
    pub descending: bool,
    pub kind: Option<ColumnType>,
}
#[derive(Clone, Copy, Debug)]
pub struct RowRef {
    pub row: u64,
    pub byte: u64,
}
pub struct View {
    pub rows: u64,
    pub index: Option<Vec<RowRef>>,
}

// Decimal comparison preserves large integer IDs beyond f64's 53-bit precision.
struct Numeric {
    sign: i8,
    order: i32,
    digits: String,
}
impl Numeric {
    fn parse(value: &str) -> Option<Self> {
        number(value)?;
        let compact: String = value
            .trim()
            .chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| if c == ',' { '.' } else { c })
            .collect();
        let (mantissa, exponent) = compact.split_once(['e', 'E']).unwrap_or((&compact, "0"));
        let negative = mantissa.starts_with('-');
        let mantissa = mantissa.trim_start_matches(['-', '+']);
        let point = mantissa.find('.').unwrap_or(mantissa.len()) as i32;
        let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
        let significant = digits.trim_start_matches('0');
        if significant.is_empty() {
            return Some(Self {
                sign: 0,
                order: 0,
                digits: String::new(),
            });
        }
        Some(Self {
            sign: if negative { -1 } else { 1 },
            order: (point - (digits.len() - significant.len()) as i32)
                .checked_add(exponent.parse::<i32>().ok()?)?,
            digits: significant.trim_end_matches('0').into(),
        })
    }
    fn compare(&self, other: &Self) -> Ordering {
        let sign = self.sign.cmp(&other.sign);
        if sign != Ordering::Equal || self.sign == 0 {
            return sign;
        }
        let magnitude = self
            .order
            .cmp(&other.order)
            .then_with(|| self.digits.cmp(&other.digits));
        if self.sign < 0 {
            magnitude.reverse()
        } else {
            magnitude
        }
    }
}
enum Key {
    Number(Numeric),
    Date(i128),
    Text(String),
    Empty,
}
fn text_key(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            // Russian alphabet places ё after е, rather than after я (Unicode order).
            'ё' => 'ж',
            'ж'..='я' => char::from_u32(c as u32 + 1).unwrap(),
            _ => c,
        })
        .collect()
}
fn compare_text(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (start_a, start_b) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let aa = &a[start_a..i];
            let bb = &b[start_b..j];
            let aa = &aa[aa.iter().position(|c| *c != b'0').unwrap_or(aa.len())..];
            let bb = &bb[bb.iter().position(|c| *c != b'0').unwrap_or(bb.len())..];
            let order = aa.len().cmp(&bb.len()).then_with(|| aa.cmp(bb));
            if order != Ordering::Equal {
                return order;
            }
        } else {
            let order = a[i].cmp(&b[j]);
            if order != Ordering::Equal {
                return order;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}
fn key(value: &str, kind: ColumnType) -> Key {
    if value.trim().is_empty() {
        return Key::Empty;
    }
    match kind {
        ColumnType::Number => Numeric::parse(value).map(Key::Number),
        ColumnType::Date => date(value).map(Key::Date),
        ColumnType::Timestamp => timestamp(value).map(Key::Date),
        ColumnType::Text => Some(Key::Text(text_key(value))),
    }
    .unwrap_or_else(|| Key::Text(text_key(value)))
}
fn compare(a: &Key, b: &Key, descending: bool) -> Ordering {
    // Missing values always sort last, including descending order.
    match (a, b) {
        (Key::Empty, Key::Empty) => return Ordering::Equal,
        (Key::Empty, _) => return Ordering::Greater,
        (_, Key::Empty) => return Ordering::Less,
        _ => {}
    }
    let order = match (a, b) {
        (Key::Number(a), Key::Number(b)) => a.compare(b),
        (Key::Date(a), Key::Date(b)) => a.cmp(b),
        (Key::Text(a), Key::Text(b)) => compare_text(a, b),
        // Invalid typed values follow valid ones in either direction.
        (Key::Text(_), _) => return Ordering::Greater,
        (_, Key::Text(_)) => return Ordering::Less,
        _ => Ordering::Equal,
    };
    if descending { order.reverse() } else { order }
}

pub fn execute(
    data: &Dataset,
    query: &Query,
    types: &[ColumnType],
    progress: &Progress,
) -> Result<View> {
    if let Some(sort) = &query.sort {
        ensure!(
            sort.column < data.headers.len(),
            "Столбец сортировки недоступен"
        );
    }
    let mut filters = Vec::new();
    for filter in &query.filters {
        ensure!(
            filter.column < data.headers.len(),
            "Столбец фильтра недоступен"
        );
        let op = match filter.op.as_str() {
            "contains" => FilterOp::Contains,
            "notContains" => FilterOp::NotContains,
            "equals" => FilterOp::Equals,
            "notEquals" => FilterOp::NotEquals,
            "startsWith" => FilterOp::StartsWith,
            "regex" => FilterOp::Regex,
            "empty" => FilterOp::Empty,
            "notEmpty" => FilterOp::NotEmpty,
            _ => bail!("Неизвестное условие фильтра"),
        };
        filters.push(CompiledFilter::new(&FilterRule {
            column: filter.column,
            op,
            text: filter.text.clone(),
            case_sensitive: query.case_sensitive,
        })?);
    }
    let regex = if query.text.is_empty() {
        None
    } else {
        Some(compile_pattern(&FindSpec {
            text: query.text.clone(),
            regex: query.regex,
            case_sensitive: query.case_sensitive,
            scope: Scope::Table,
        })?)
    };
    progress.check()?;
    if query.sort.is_none() && filters.is_empty() && regex.is_none() {
        return Ok(View {
            rows: data.rows,
            index: None,
        });
    }
    progress.total.store(data.rows, AtomicOrdering::Relaxed);
    let mut index = Vec::new();
    let mut keys = Vec::new();
    let sort_type = query
        .sort
        .as_ref()
        .map(|sort| sort.kind.unwrap_or(types[sort.column]));
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 8)
        .min(data.rows.div_ceil(4096).max(1) as usize);
    let shards = std::thread::scope(|scope| -> Result<Vec<_>> {
        let mut handles = Vec::new();
        for worker in 0..workers {
            let begin = data.rows * worker as u64 / workers as u64;
            let end = data.rows * (worker + 1) as u64 / workers as u64;
            let regex = regex.clone();
            let filters = filters.clone();
            handles.push(scope.spawn(move || -> Result<_> {
                let mut index = Vec::new();
                let mut keys = Vec::new();
                if begin == end {
                    return Ok((index, keys));
                }
                let mut reader = data.reader_at(begin)?;
                let mut record = ByteRecord::new();
                for row in begin..end {
                    if (row - begin).is_multiple_of(1024) {
                        progress.check()?;
                        if row > begin {
                            progress.done.fetch_add(1024, AtomicOrdering::Relaxed);
                        }
                    }
                    let byte = reader.position().byte();
                    ensure!(
                        reader.read_byte_record(&mut record)?,
                        "Неожиданный конец файла"
                    );
                    if !filters.iter().all(|f| f.matches(&record)) {
                        continue;
                    }
                    if regex.as_ref().is_some_and(|r| {
                        !record
                            .iter()
                            .any(|f| r.is_match(std::str::from_utf8(f).unwrap_or("")))
                    }) {
                        continue;
                    }
                    let reference = RowRef { row, byte };
                    if let Some(sort) = &query.sort {
                        keys.push((
                            key(
                                std::str::from_utf8(record.get(sort.column).unwrap_or_default())
                                    .unwrap_or(""),
                                sort_type.unwrap(),
                            ),
                            reference,
                        ));
                    } else {
                        index.push(reference);
                    }
                }
                progress
                    .done
                    .fetch_add((end - begin - 1) % 1024 + 1, AtomicOrdering::Relaxed);
                Ok((index, keys))
            }));
        }
        let mut results = Vec::new();
        let mut error = None;
        // Join every reader, even after cancellation or an I/O error.
        for handle in handles {
            match handle
                .join()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Сбой потока обработки")))
            {
                Ok(shard) => results.push(shard),
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
        Ok(results)
    })?;
    // Shards are concatenated in source order; equal sort keys retain this order.
    if query.sort.is_some() {
        keys.reserve(shards.iter().map(|(_, keys)| keys.len()).sum());
    } else {
        index.reserve(shards.iter().map(|(index, _)| index.len()).sum());
    }
    for (mut shard_index, mut shard_keys) in shards {
        index.append(&mut shard_index);
        keys.append(&mut shard_keys);
    }
    progress.check()?;
    if let Some(sort) = &query.sort {
        keys.sort_unstable_by(|(a, ar), (b, br)| {
            compare(a, b, sort.descending).then_with(|| ar.row.cmp(&br.row))
        });
        progress.check()?;
        index = keys.into_iter().map(|(_, reference)| reference).collect();
    }
    progress.done.store(data.rows, AtomicOrdering::Relaxed);
    Ok(View {
        rows: index.len() as u64,
        index: Some(index),
    })
}

#[derive(Serialize)]
pub struct VisibleRow {
    pub id: u64,
    pub values: Vec<String>,
}

impl View {
    /// Exact offsets keep random access fast after sorting. No sparse-index rescan per cell.
    pub fn records(
        &self,
        data: &Dataset,
        start: u64,
        count: usize,
    ) -> Result<Vec<(u64, ByteRecord)>> {
        if start >= self.rows || count == 0 {
            return Ok(Vec::new());
        }
        let end = start.saturating_add(count as u64).min(self.rows);
        let mut output = Vec::with_capacity((end - start) as usize);
        if let Some(index) = &self.index {
            let mut reader = csv::ReaderBuilder::new()
                .delimiter(data.delimiter)
                .has_headers(false)
                .flexible(true)
                .buffer_capacity(8 * 1024)
                .from_reader(crate::data::open_read(&data.path)?);
            for reference in &index[start as usize..end as usize] {
                let mut position = Position::new();
                position.set_byte(reference.byte);
                reader.seek(position)?;
                let mut record = ByteRecord::new();
                ensure!(reader.read_byte_record(&mut record)?, "Строка недоступна");
                output.push((reference.row, record));
            }
        } else {
            let mut reader = data.reader_at(start)?;
            for row in start..end {
                let mut record = ByteRecord::new();
                ensure!(reader.read_byte_record(&mut record)?, "Строка недоступна");
                output.push((row, record));
            }
        }
        Ok(output)
    }
    pub fn viewport(
        &self,
        data: &Dataset,
        start: u64,
        count: usize,
        columns: &[usize],
    ) -> Result<Vec<VisibleRow>> {
        ensure!(
            count <= 256 && columns.len() <= 128,
            "Слишком большой запрос отображения"
        );
        ensure!(
            columns.iter().all(|c| *c < data.headers.len()),
            "Столбец недоступен"
        );
        Ok(self
            .records(data, start, count)?
            .into_iter()
            .map(|(id, record)| VisibleRow {
                id,
                values: columns
                    .iter()
                    .map(|c| {
                        preview(
                            std::str::from_utf8(record.get(*c).unwrap_or_default()).unwrap_or(""),
                            512,
                        )
                    })
                    .collect(),
            })
            .collect())
    }
    pub fn export(&self, data: &Dataset, path: &Path, progress: &Progress) -> Result<u64> {
        self.export_with_headers(data, path, progress, &data.headers)
    }
    pub fn export_with_headers(
        &self,
        data: &Dataset,
        path: &Path,
        progress: &Progress,
        headers: &[String],
    ) -> Result<u64> {
        ensure!(headers.len() == data.headers.len(), "Столбец недоступен");
        ensure!(
            path != data.original_path && path != data.path,
            "Нельзя перезаписать исходный файл"
        );
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temp = NamedTempFile::new_in(parent)?;
        progress.total.store(self.rows, AtomicOrdering::Relaxed);
        {
            let mut writer = csv::Writer::from_writer(temp.as_file_mut());
            writer.write_record(headers)?;
            for start in (0..self.rows).step_by(1024) {
                progress.check()?;
                for (_, mut record) in self.records(data, start, 1024)? {
                    while record.len() < data.headers.len() {
                        record.push_field(b"");
                    }
                    writer.write_byte_record(&record)?;
                }
                progress
                    .done
                    .store((start + 1024).min(self.rows), AtomicOrdering::Relaxed);
            }
            writer.flush()?;
        }
        progress.check()?;
        temp.persist(path).context("Не удалось сохранить CSV")?;
        Ok(self.rows)
    }
}
