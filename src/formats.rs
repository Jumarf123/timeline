//! Read-only adapters. Imported records are normalized to disk, never to a DOM-sized array.
use crate::data::{Progress, open_read};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read},
    path::Path,
    sync::atomic::Ordering,
};
use tempfile::NamedTempFile;

pub struct Normalized {
    pub file: NamedTempFile,
    pub headers: Vec<String>,
    pub warnings: Vec<String>,
}
pub(crate) struct Rows {
    file: NamedTempFile,
    writer: csv::Writer<std::fs::File>,
    headers: Vec<String>,
    columns: HashMap<String, usize>,
    warnings: Vec<String>,
}
impl Rows {
    pub(crate) fn new(names: &[&str]) -> Result<Self> {
        let file = NamedTempFile::new()?;
        let writer = csv::WriterBuilder::new()
            .flexible(true)
            .buffer_capacity(1024 * 1024)
            .from_writer(file.reopen()?);
        Ok(Self {
            file,
            writer,
            headers: names.iter().map(|s| s.to_string()).collect(),
            columns: names
                .iter()
                .enumerate()
                .map(|(i, s)| (s.to_string(), i))
                .collect(),
            warnings: vec![],
        })
    }
    pub(crate) fn fields(&mut self, fields: Vec<(String, String)>) -> Result<()> {
        let mut row = vec![String::new(); self.headers.len()];
        for (name, text) in fields {
            let index = *self.columns.entry(name.clone()).or_insert_with(|| {
                let i = self.headers.len();
                self.headers.push(name);
                i
            });
            ensure!(index < 16_384, "Too many columns in structured file");
            row.resize(row.len().max(index + 1), String::new());
            row[index] = text;
        }
        self.writer.write_record(row)?;
        Ok(())
    }
    pub(crate) fn finish(mut self) -> Result<Normalized> {
        self.writer.flush()?;
        Ok(Normalized {
            file: self.file,
            headers: self.headers,
            warnings: self.warnings,
        })
    }
}
fn flatten(value: Value, path: &str, fields: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, value) in map {
                flatten(
                    value,
                    &format!("{path}/{}", key.replace('~', "~0").replace('/', "~1")),
                    fields,
                );
            }
        }
        Value::String(s) => fields.push((path.into(), s)),
        other => fields.push((path.into(), other.to_string())),
    }
}

pub fn source_kind(extension: &str) -> Option<&'static str> {
    Some(match extension {
        "md" | "markdown" => "markdown",
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => "javascript",
        "py" | "pyw" => "python",
        "ps1" | "psm1" | "psd1" => "powershell",
        "sh" | "bash" => "shell",
        "yaml" | "yml" => "yaml",
        "ini" | "cfg" | "conf" | "reg" | "toml" | "rs" | "c" | "cpp" | "h" | "cs" | "sql"
        | "bat" | "cmd" => "text",
        _ => return None,
    })
}
pub fn is_binary(sample: &[u8]) -> bool {
    sample.contains(&0)
        || sample
            .iter()
            .filter(|&&b| b < 9 || (b > 13 && b < 32))
            .count()
            * 100
            > sample.len().max(1)
}
/// A ruler line plus a named header is much stronger evidence than spaces alone.
pub fn text_table(sample: &[u8]) -> Option<Vec<String>> {
    let text = std::str::from_utf8(sample).ok()?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next()?.trim_start_matches('\u{feff}');
    let ruler = lines.next()?.trim();
    if ruler.len() < 5 || !ruler.chars().all(|c| matches!(c, '-' | '=' | ' ' | '+')) {
        return None;
    }
    let headers: Vec<_> = header.split_whitespace().map(str::to_owned).collect();
    if !(2..=64).contains(&headers.len()) {
        return None;
    }
    let sample: Vec<_> = lines.take(16).collect();
    if sample.is_empty()
        || sample
            .iter()
            .any(|line| line.split_whitespace().count() < headers.len())
    {
        return None;
    }
    Some(headers)
}
pub fn normalize_text_table(
    path: &Path,
    headers: Vec<String>,
    progress: &Progress,
) -> Result<Normalized> {
    let mut rows = Rows::new(&[])?;
    rows.headers = headers;
    let mut skip = 2;
    for line in BufReader::new(open_read(path)?).lines() {
        progress.check()?;
        let line = line?;
        progress
            .done
            .fetch_add(line.len() as u64 + 1, Ordering::Relaxed);
        if line.trim().is_empty() {
            continue;
        }
        if skip > 0 {
            skip -= 1;
            continue;
        }
        let mut tail = line.trim();
        let mut fields = Vec::new();
        for _ in 1..rows.headers.len() {
            let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
            fields.push(&tail[..end]);
            tail = tail[end..].trim_start();
        }
        fields.push(tail);
        rows.writer.write_record(fields)?;
        progress.records.fetch_add(1, Ordering::Relaxed);
    }
    rows.finish()
}
pub fn hex(path: &Path, progress: &Progress) -> Result<Normalized> {
    let mut rows = Rows::new(&["Offset", "Hex", "Text"])?;
    let mut input = BufReader::with_capacity(1024 * 1024, open_read(path)?);
    let mut offset = 0u64;
    loop {
        progress.check()?;
        let mut bytes = [0; 32];
        let mut size = 0;
        while size < bytes.len() {
            let n = input.read(&mut bytes[size..])?;
            if n == 0 {
                break;
            }
            size += n;
        }
        if size == 0 {
            break;
        }
        let text: String = bytes[..size]
            .iter()
            .map(|b| {
                if (32..127).contains(b) {
                    *b as char
                } else {
                    '.'
                }
            })
            .collect();
        rows.writer
            .write_record([format!("{offset:016X}"), hex_bytes(&bytes[..size]), text])?;
        offset += size as u64;
        progress.records.fetch_add(1, Ordering::Relaxed);
        progress.done.store(offset, Ordering::Relaxed);
    }
    rows.warnings.push(
        "Binary file: byte offsets, hexadecimal data and ASCII. No format-specific interpretation."
            .into(),
    );
    rows.finish()
}
fn hex_bytes(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 3);
    for b in bytes {
        let _ = write!(s, "{b:02X} ");
    }
    s.trim_end().into()
}

/// A bounded, cancellable strings view of an otherwise unknown binary file.
/// The original bytes remain available through the explicit Hex format.
pub fn binary_readable(path: &Path, progress: &Progress) -> Result<Normalized> {
    const BLOCK: usize = 1024 * 1024;
    const OVERLAP: usize = 64 * 1024;
    let input = open_read(path)?;
    let bytes = input.metadata()?.len();
    progress.begin("parse", bytes, "bytes");
    let mut input = BufReader::with_capacity(BLOCK, input);
    let mut rows = Rows::new(&["Offset", "Length", "Encoding", "Text"])?;
    let mut buffer = Vec::with_capacity(BLOCK + OVERLAP);
    let mut offset = 0u64;
    let mut eof = false;
    let mut count = 0u64;
    let mut split = false;
    loop {
        progress.check()?;
        while buffer.len() < BLOCK + OVERLAP && !eof {
            let start = buffer.len();
            buffer.resize(BLOCK + OVERLAP, 0);
            let n = input.read(&mut buffer[start..])?;
            buffer.truncate(start + n);
            eof = n == 0;
            progress.check()?;
        }
        if buffer.is_empty() {
            break;
        }
        let spans = crate::binary::spans(&buffer);
        let mut consumed = if eof { buffer.len() } else { BLOCK };
        if !eof {
            // Keep a short crossing span intact. A span larger than half a
            // block is emitted at a character boundary to bound memory use.
            for span in &spans {
                let end = span.offset + span.len;
                if span.offset < consumed && end > consumed {
                    if span.offset >= BLOCK / 2 {
                        consumed = span.offset;
                    } else {
                        consumed = end;
                        split |= end == buffer.len() || buffer.len() - end <= 3;
                    }
                    break;
                }
            }
        }
        for span in spans {
            progress.check()?;
            if span.offset + span.len > consumed {
                continue;
            }
            rows.writer.write_record([
                format!("{:016X}", offset + span.offset as u64),
                span.len.to_string(),
                span.encoding.to_string(),
                span.text,
            ])?;
            count += 1;
            progress.records.store(count, Ordering::Relaxed);
        }
        buffer.drain(..consumed);
        offset += consumed as u64;
        progress.done.store(offset, Ordering::Relaxed);
    }
    if count == 0 {
        rows.writer.write_record([
            "0000000000000000".into(),
            bytes.to_string(),
            "Binary".into(),
            "No confidently readable text was found. Open as Hex to inspect all original bytes."
                .into(),
        ])?;
        progress.records.store(1, Ordering::Relaxed);
    }
    rows.warnings.push("Binary strings view: probable UTF-8/UTF-16 text with original byte offsets, not a complete interpretation of this file format. Open as Hex to inspect every original byte, including unrecognized data.".into());
    if split {
        rows.warnings.push(
            "Very long text spans are split at read-window boundaries to keep memory use bounded."
                .into(),
        );
    }
    rows.finish()
}

fn escaped_column(name: &str) -> String {
    name.replace('~', "~0").replace('/', "~1")
}

fn binary_fields(name: &str, bytes: &[u8], fields: &mut Vec<(String, String)>) {
    let raw = hex_bytes(bytes);
    let value = crate::binary::readable(bytes).unwrap_or_else(|| raw.clone());
    let name = escaped_column(name);
    fields.push((format!("/{name}"), value));
    fields.push((format!("$raw/{name}"), raw));
}

pub fn evtx(path: &Path, progress: &Progress) -> Result<Normalized> {
    use rayon::prelude::*;
    use std::io::{Seek, SeekFrom};
    use std::sync::Arc;
    const CHUNK: usize = 65536;
    progress.check()?;
    let bytes = open_read(path)?.metadata()?.len();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .min(32)
        .min(bytes.saturating_sub(4096).div_ceil(CHUNK as u64).max(1) as usize);
    let settings = Arc::new(evtx::ParserSettings::default());
    let mut input = BufReader::with_capacity(CHUNK * threads, open_read(path)?);
    // Validate the file header, then process bounded batches of physical chunks.
    // Physical traversal also retains errors in trailing, incomplete chunks.
    evtx::EvtxFileHeader::from_stream(&mut input)?;
    input.seek(SeekFrom::Start(4096))?;
    progress.begin("parse", bytes, "bytes");
    progress.done.store(4096, Ordering::Relaxed);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()?;
    let mut arenas: Vec<_> = (0..threads)
        .map(|_| bumpalo::Bump::with_capacity(CHUNK))
        .collect();
    let mut rows = Rows::new(&["Record ID", "Timestamp"])?;
    let mut errors = 0u64;
    let mut offset = 4096u64;
    while offset < bytes {
        progress.check()?;
        let mut batch = Vec::with_capacity(threads);
        for _ in 0..threads {
            if offset >= bytes {
                break;
            }
            progress.check()?;
            let length = (bytes - offset).min(CHUNK as u64) as usize;
            let mut data = vec![0; length];
            input.read_exact(&mut data)?;
            batch.push((offset, data, arenas.pop().unwrap_or_default()));
            offset += length as u64;
        }
        let results: Vec<_> = pool.install(|| {
            batch
                .into_par_iter()
                .map(|(offset, data, arena)| {
                    let length = data.len();
                    let mut output = Vec::new();
                    let mut error_count = 0u64;
                    let mut arena = Some(arena);
                    let result = (|| -> Result<()> {
                        progress.check()?;
                        if length == CHUNK && data.iter().all(|&b| b == 0) {
                            return Ok(());
                        }
                        ensure!(
                            length == CHUNK,
                            "Incomplete EVTX chunk at byte {offset}: {length} bytes"
                        );
                        let mut chunk = evtx::EvtxChunkData::new(data, false)?;
                        let mut parsed =
                            chunk.parse_with_arena(settings.clone(), arena.take().unwrap())?;
                        for record in parsed.iter() {
                            progress.check()?;
                            let mut fields = Vec::new();
                            let result = record.and_then(|record| {
                                fields.extend([
                                    ("Record ID".into(), record.event_record_id.to_string()),
                                    ("Timestamp".into(), record.timestamp.to_string()),
                                ]);
                                record.into_json_value()
                            });
                            match result {
                                Ok(record) => flatten(record.data, "", &mut fields),
                                Err(error) => {
                                    error_count += 1;
                                    fields.push((
                                        "Parse error".into(),
                                        format!("Chunk at {offset}: {error}"),
                                    ));
                                }
                            }
                            output.push(fields);
                            progress.records.fetch_add(1, Ordering::Relaxed);
                        }
                        arena = Some(parsed.into_arena());
                        Ok(())
                    })();
                    if let Err(error) = result {
                        error_count += 1;
                        output.push(vec![(
                            "Parse error".into(),
                            format!("Chunk at {offset}: {error:#}"),
                        )]);
                        progress.records.fetch_add(1, Ordering::Relaxed);
                    }
                    progress.done.fetch_add(length as u64, Ordering::Relaxed);
                    (output, error_count, arena.unwrap_or_default())
                })
                .collect()
        });
        // Indexed parallel collection preserves source order, including duplicates.
        for (records, count, arena) in results {
            progress.check()?;
            errors += count;
            arenas.push(arena);
            for fields in records {
                progress.check()?;
                rows.fields(fields)?;
            }
        }
    }
    if errors > 0 {
        rows.warnings.push(format!(
            "EVTX: {errors} damaged records/chunks. Errors are included as rows."
        ));
    }
    rows.finish()
}

pub fn xml(path: &Path, progress: &Progress) -> Result<Normalized> {
    use quick_xml::{Reader, events::Event};
    let mut reader = Reader::from_reader(BufReader::new(open_read(path)?));
    let mut buffer = Vec::new();
    let mut stack: Vec<(String, HashMap<String, usize>)> = Vec::new();
    let mut rows = Rows::new(&["Path", "Value", "Kind"])?;
    let mut text = String::new();
    let mut has_doctype = false;
    loop {
        progress.check()?;
        let event = reader.read_event_into(&mut buffer).context("Invalid XML")?;
        let empty = matches!(event, Event::Empty(_));
        if !matches!(
            event,
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_)
        ) {
            if !text.is_empty() {
                rows.writer.write_record([
                    stack.last().map(|(p, _)| p.as_str()).unwrap_or("/"),
                    text.as_str(),
                    if text.trim().is_empty() {
                        "whitespace"
                    } else {
                        "text"
                    },
                ])?;
                progress.records.fetch_add(1, Ordering::Relaxed);
            }
            text.clear();
        }
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let name = reader.decoder().decode(e.name().as_ref())?.into_owned();
                let index = if let Some((_, counts)) = stack.last_mut() {
                    let n = counts.entry(name.clone()).or_default();
                    *n += 1;
                    *n
                } else {
                    1
                };
                let path = format!(
                    "{}/{}[{}]",
                    stack.last().map(|(p, _)| p.as_str()).unwrap_or(""),
                    name,
                    index
                );
                rows.writer.write_record([path.as_str(), "", "element"])?;
                progress.records.fetch_add(1, Ordering::Relaxed);
                for attr in e.attributes() {
                    let attr = attr?;
                    let raw_value = reader.decoder().decode(&attr.value)?;
                    let value = xml_unescape(&raw_value, has_doctype)?;
                    rows.writer.write_record([
                        format!("{path}/@{}", reader.decoder().decode(attr.key.as_ref())?),
                        value,
                        "attribute".into(),
                    ])?;
                    progress.records.fetch_add(1, Ordering::Relaxed);
                }
                if !empty {
                    stack.push((path, HashMap::new()));
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Text(e) => {
                text.push_str(&e.decode()?);
            }
            Event::CData(e) => {
                text.push_str(&e.decode()?);
            }
            Event::GeneralRef(e) => {
                let reference = format!("&{};", e.decode()?);
                text.push_str(&xml_unescape(&reference, has_doctype)?);
            }
            Event::DocType(e) => {
                rows.writer.write_record(["/", &e.decode()?, "doctype"])?;
                progress.records.fetch_add(1, Ordering::Relaxed);
                if !has_doctype {
                    rows.warnings.push("XML DTD is preserved as text. Custom entity references remain literal; external resources are never loaded.".into());
                }
                has_doctype = true;
            }
            Event::Comment(e) => {
                rows.writer.write_record([
                    stack.last().map(|(p, _)| p.as_str()).unwrap_or("/"),
                    &e.decode()?,
                    "comment",
                ])?;
                progress.records.fetch_add(1, Ordering::Relaxed);
            }
            Event::PI(e) => {
                rows.writer.write_record([
                    stack.last().map(|(p, _)| p.as_str()).unwrap_or("/"),
                    &reader.decoder().decode(e.as_ref())?,
                    "processing instruction",
                ])?;
                progress.records.fetch_add(1, Ordering::Relaxed);
            }
            Event::Decl(e) => {
                rows.writer.write_record([
                    "/",
                    &reader.decoder().decode(e.as_ref())?,
                    "declaration",
                ])?;
                progress.records.fetch_add(1, Ordering::Relaxed);
            }
            Event::Eof => {
                ensure!(stack.is_empty(), "Unclosed XML element");
                break;
            }
        }
        progress
            .done
            .store(reader.buffer_position(), Ordering::Relaxed);
        buffer.clear();
    }
    rows.finish()
}

fn xml_unescape(raw: &str, literal_unknown: bool) -> Result<String> {
    let mut output = String::with_capacity(raw.len());
    let mut tail = raw;
    while let Some(start) = tail.find('&') {
        output.push_str(&tail[..start]);
        let reference = &tail[start..];
        let end = reference.find(';').context("Unterminated XML entity")? + 1;
        match quick_xml::escape::unescape(&reference[..end]) {
            Ok(value) => output.push_str(&value),
            Err(quick_xml::escape::EscapeError::UnrecognizedEntity(_, _)) if literal_unknown => {
                output.push_str(&reference[..end]);
            }
            Err(error) => return Err(error.into()),
        }
        tail = &reference[end..];
    }
    output.push_str(tail);
    Ok(output)
}

pub fn registry(path: &Path, progress: &Progress) -> Result<Normalized> {
    progress.begin("parse", 0, "records");
    progress.check()?;
    use notatin::parser_builder::ParserBuilder;
    // The builder stores this path behind a 'static trait object.
    let owned_path = path.to_path_buf();
    let parser = ParserBuilder::from_path(owned_path)
        .recover_deleted(false)
        .build()?;
    let mut rows = Rows::new(&[
        "Key",
        "Name",
        "Type",
        "Value",
        "Last write",
        "Offset",
        "Parse warning",
        "Raw data",
    ])?;
    if parser.get_parse_logs().has_logs() {
        rows.warnings
            .push(format!("Registry parser: {}", parser.get_parse_logs()));
    }
    for key in notatin::parser::ParserIterator::new(&parser) {
        progress.check()?;
        let date = key.last_key_written_date_and_time().to_rfc3339();
        let mut count = 0;
        for value in key.value_iter() {
            progress.check()?;
            count += 1;
            let (content, content_logs) = value.get_content();
            let warning = format!(
                "{}{}{}",
                key.logs,
                value.logs,
                content_logs
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default()
            );
            let (text, raw) = match &content {
                notatin::cell_value::CellValue::Binary(bytes) => {
                    let raw = hex_bytes(bytes);
                    (
                        crate::binary::readable(bytes).unwrap_or_else(|| raw.clone()),
                        raw,
                    )
                }
                notatin::cell_value::CellValue::MultiString(values) => (
                    values.join("\n"),
                    value
                        .detail
                        .value_bytes()
                        .as_ref()
                        .map(|bytes| hex_bytes(bytes))
                        .unwrap_or_default(),
                ),
                _ => {
                    let raw = if warning.is_empty() {
                        String::new()
                    } else {
                        value
                            .detail
                            .value_bytes()
                            .as_ref()
                            .map(|bytes| hex_bytes(bytes))
                            .unwrap_or_default()
                    };
                    (content.to_string(), raw)
                }
            };
            rows.writer.write_record([
                key.path.clone(),
                value.get_pretty_name(),
                format!("{:?}", value.data_type),
                text,
                date.clone(),
                format!("0x{:X}", value.file_offset_absolute),
                warning,
                raw,
            ])?;
        }
        if count == 0 {
            rows.writer.write_record([
                key.path.clone(),
                String::new(),
                "Key".into(),
                String::new(),
                date,
                format!("0x{:X}", key.file_offset_absolute),
                key.logs.to_string(),
                String::new(),
            ])?;
        }
        progress.records.fetch_add(count.max(1), Ordering::Relaxed);
    }
    rows.warnings
        .push("Registry hive snapshot; transaction logs and deleted keys are not replayed.".into());
    rows.warnings.push("Binary values show probable readable strings when present; Raw data retains the original hexadecimal bytes.".into());
    rows.finish()
}

pub fn ese(path: &Path, progress: &Progress) -> Result<Normalized> {
    progress.begin("parse", 0, "records");
    #[cfg(windows)]
    let native_error = match crate::ese_native::read(path, progress) {
        Ok(result) => return Ok(result),
        Err(error) => format!(
            "Native ESENT could not read this snapshot: {error:#}. Using the portable parser."
        ),
    };
    progress.check()?;
    let db = ese_core::EseDatabase::open(path)?;
    let entries = db.catalog_entries()?;
    let tables: Vec<_> = entries.iter().filter(|e| e.object_type == 1).collect();
    ensure!(
        !tables.is_empty(),
        "ESE catalog contains no readable tables"
    );
    let mut rows = Rows::new(&["Table", "Page", "Record"])?;
    #[cfg(windows)]
    rows.warnings.push(native_error);
    for table in tables {
        progress.check()?;
        let columns = db.table_columns(&table.object_name)?;
        for row in db.table_records_from_root(table.table_page)? {
            progress.check()?;
            let (page, tag, raw) = row?;
            let values = if db.is_extended_format() {
                ese_core::decode_ese_record(&raw, &columns, true)
            } else {
                ese_core::decode_record(&raw, &columns)
            }?;
            let mut fields = vec![
                ("Table".into(), table.object_name.clone()),
                ("Page".into(), page.to_string()),
                ("Record".into(), tag.to_string()),
            ];
            for (name, value) in values {
                match value {
                    ese_core::EseValue::Binary(bytes) => binary_fields(&name, &bytes, &mut fields),
                    other => fields.push((format!("/{}", escaped_column(&name)), ese_text(other))),
                }
            }
            rows.fields(fields)?;
            progress.records.fetch_add(1, Ordering::Relaxed);
        }
    }
    rows.warnings.push("ESE snapshot: transaction logs are not replayed. Separated/compressed long values may remain binary. Binary values show probable readable strings when present; $raw columns retain their original hexadecimal bytes.".into());
    rows.finish()
}

pub fn sqlite(path: &Path, progress: &Progress) -> Result<Normalized> {
    progress.begin("parse", 0, "records");
    use rusqlite::{Connection, OpenFlags, types::ValueRef};
    // Read-only + immutable URI: no journal creation and no changes to evidence.
    let uri = format!(
        "file:{}?immutable=1",
        path.to_string_lossy()
            .replace('\\', "/")
            .replace('%', "%25")
            .replace('?', "%3F")
            .replace('#', "%23")
    );
    let db = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let tables=db.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut output = Rows::new(&["Table"])?;
    for table in tables {
        progress.check()?;
        let mut query = db.prepare(&format!("SELECT * FROM \"{}\"", table.replace('"', "\"\"")))?;
        let names: Vec<_> = query.column_names().iter().map(|n| n.to_string()).collect();
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            progress.check()?;
            let mut fields = vec![("Table".into(), table.clone())];
            for (i, name) in names.iter().enumerate() {
                let value = match row.get_ref(i)? {
                    ValueRef::Null => String::new(),
                    ValueRef::Integer(n) => n.to_string(),
                    ValueRef::Real(n) => n.to_string(),
                    ValueRef::Text(b) => match std::str::from_utf8(b) {
                        Ok(text) => text.to_owned(),
                        Err(_) => {
                            binary_fields(name, b, &mut fields);
                            continue;
                        }
                    },
                    ValueRef::Blob(b) => {
                        binary_fields(name, b, &mut fields);
                        continue;
                    }
                };
                fields.push((format!("/{}", escaped_column(name)), value));
            }
            output.fields(fields)?;
            progress.records.fetch_add(1, Ordering::Relaxed);
        }
    }
    output
        .warnings
        .push("SQLite main database snapshot; separate WAL files are not applied. Binary values show probable readable strings when present; $raw columns retain their original hexadecimal bytes.".into());
    output.finish()
}
fn ese_text(value: ese_core::EseValue) -> String {
    use ese_core::EseValue::*;
    match value {
        Null => String::new(),
        Text(s) => s,
        Binary(v) => crate::binary::readable(&v).unwrap_or_else(|| hex_bytes(&v)),
        Guid(v) => format!(
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            u32::from_le_bytes(v[..4].try_into().unwrap()),
            u16::from_le_bytes(v[4..6].try_into().unwrap()),
            u16::from_le_bytes(v[6..8].try_into().unwrap()),
            v[8],
            v[9],
            v[10],
            v[11],
            v[12],
            v[13],
            v[14],
            v[15]
        ),
        DateTime(days) => {
            // OLE DATE fractions are positive times of day even before its epoch.
            let seconds = (days.trunc() - 25569.0) * 86400.0 + days.fract().abs() * 86400.0;
            if seconds.is_finite() && seconds.abs() < i64::MAX as f64 {
                let whole = seconds.floor();
                let nanos = ((seconds - whole) * 1_000_000_000.0).round() as u32;
                if let Some(date) = chrono::DateTime::from_timestamp(
                    whole as i64 + i64::from(nanos == 1_000_000_000),
                    nanos % 1_000_000_000,
                ) {
                    return date.to_rfc3339();
                }
            }
            days.to_string()
        }
        other => {
            let value = serde_json::to_value(other).unwrap_or_default();
            value
                .as_object()
                .and_then(|m| m.values().next())
                .map(|v| v.to_string())
                .unwrap_or_default()
        }
    }
}

pub fn usn(path: &Path, progress: &Progress) -> Result<Normalized> {
    let mut input = BufReader::with_capacity(1024 * 1024, open_read(path)?);
    let mut rows = Rows::new(&[
        "Name",
        "Timestamp",
        "USN",
        "Reason",
        "File reference",
        "Parent reference",
        "Version",
        "Offset",
    ])?;
    let mut offset = 0u64;
    loop {
        progress.check()?;
        let mut head = [0; 8];
        let mut size = 0;
        while size < 8 {
            let n = input.read(&mut head[size..])?;
            if n == 0 {
                break;
            }
            size += n;
        }
        if size == 0 {
            break;
        }
        ensure!(size == 8, "Truncated USN header at {offset}");
        if head == [0; 8] {
            offset += 8;
            progress.done.store(offset, Ordering::Relaxed);
            continue;
        }
        let length = u32::from_le_bytes(head[..4].try_into().unwrap()) as usize;
        let version = u16::from_le_bytes(head[4..6].try_into().unwrap());
        ensure!(
            (60..=1024 * 1024).contains(&length) && length.is_multiple_of(8),
            "Invalid USN record length at {offset}"
        );
        let mut data = vec![0; length];
        data[..8].copy_from_slice(&head);
        input.read_exact(&mut data[8..])?;
        let u16_at = |i| u16::from_le_bytes(data[i..i + 2].try_into().unwrap()) as usize;
        let u64_at = |i| u64::from_le_bytes(data[i..i + 8].try_into().unwrap());
        let (usn_at, time_at, reason_at, name_at, name_length, file, parent) = match version {
            2 => (
                24,
                32,
                40,
                u16_at(58),
                u16_at(56),
                u64_at(8).to_string(),
                u64_at(16).to_string(),
            ),
            3 if length >= 76 => (
                40,
                48,
                56,
                u16_at(74),
                u16_at(72),
                hex_bytes(&data[8..24]),
                hex_bytes(&data[24..40]),
            ),
            _ => {
                rows.writer.write_record([
                    "".into(),
                    "".into(),
                    "".into(),
                    "Unsupported record version; raw data: ".to_owned() + &hex_bytes(&data),
                    "".into(),
                    "".into(),
                    version.to_string(),
                    offset.to_string(),
                ])?;
                offset += length as u64;
                progress.records.fetch_add(1, Ordering::Relaxed);
                progress.done.store(offset, Ordering::Relaxed);
                continue;
            }
        };
        ensure!(
            name_length % 2 == 0 && name_at >= 60 && name_at + name_length <= length,
            "Invalid USN filename at {offset}"
        );
        let name = String::from_utf16_lossy(
            &data[name_at..name_at + name_length]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect::<Vec<_>>(),
        );
        let ticks = u64_at(time_at);
        let seconds = (i128::from(ticks) / 10_000_000 - 11_644_473_600) as i64;
        let date = chrono::DateTime::from_timestamp(seconds, ((ticks % 10_000_000) * 100) as u32)
            .map(|d| d.to_rfc3339())
            .unwrap_or_default();
        let reason = u32::from_le_bytes(data[reason_at..reason_at + 4].try_into().unwrap());
        let reasons = [
            (1, "DATA_OVERWRITE"),
            (2, "DATA_EXTEND"),
            (4, "DATA_TRUNCATION"),
            (0x100, "FILE_CREATE"),
            (0x200, "FILE_DELETE"),
            (0x1000, "RENAME_OLD_NAME"),
            (0x2000, "RENAME_NEW_NAME"),
            (0x8000, "BASIC_INFO_CHANGE"),
            (0x10000, "HARD_LINK_CHANGE"),
            (0x80000000, "CLOSE"),
        ];
        let labels: Vec<_> = reasons
            .iter()
            .filter(|(flag, _)| reason & flag != 0)
            .map(|(_, s)| *s)
            .collect();
        rows.writer.write_record([
            name,
            date,
            u64_at(usn_at).to_string(),
            format!("0x{reason:08X} {}", labels.join(" | ")),
            file,
            parent,
            version.to_string(),
            offset.to_string(),
        ])?;
        offset += length as u64;
        progress.records.fetch_add(1, Ordering::Relaxed);
        progress.done.store(offset, Ordering::Relaxed);
    }
    rows.finish()
}

pub fn arn(path: &Path, progress: &Progress) -> Result<Normalized> {
    use std::collections::{BTreeMap, HashSet};
    progress.check()?;
    let source = open_read(path)?;
    // The source (or its acquired snapshot) denies writes/deletes on Windows.
    // Mapping avoids tens of thousands of tiny seek/read system calls in CFB.
    let mapped = unsafe { memmap2::MmapOptions::new().map(&source)? };
    let mut compound = cfb::CompoundFile::open(std::io::Cursor::new(&mapped[..]))?;
    let mut header = Vec::new();
    compound
        .open_stream("/Header")?
        .take(4096)
        .read_to_end(&mut header)?;
    ensure!(
        header.len() >= 4
            && arn_string(&header[..header.len() - 4]).is_some_and(|s| s == "Autoruns"),
        "Compound file is not an Autoruns snapshot"
    );
    let streams: Vec<_> = compound
        .walk()
        .filter(|e| e.is_stream())
        .map(|e| (e.path().to_owned(), e.len()))
        .collect();
    let mut groups = BTreeMap::<(u64, String), Vec<(String, std::path::PathBuf, u64)>>::new();
    let mut field_names = HashSet::new();
    for (path, length) in streams {
        progress.check()?;
        let parts: Vec<_> = path
            .components()
            .filter_map(|s| {
                if let std::path::Component::Normal(name) = s {
                    name.to_str()
                } else {
                    None
                }
            })
            .collect();
        if parts.len() >= 3 && parts[0].eq_ignore_ascii_case("Items") {
            let id = parts[1].parse::<u64>().unwrap_or(u64::MAX);
            let name = parts[2..].join("/");
            field_names.insert(name.clone());
            groups
                .entry((id, parts[1].to_owned()))
                .or_default()
                .push((name, path, length));
        }
    }
    // Keep unknown fields verbatim, even if a future version uses our display
    // column names. Only the derived columns receive a disambiguating suffix.
    let original_fields: Vec<_> = field_names.iter().cloned().collect();
    let mut derived_name = |name: &str| {
        let mut candidate = name.to_owned();
        let mut suffix = 1;
        while !field_names.insert(candidate.clone()) {
            candidate = format!("{name} (inferred {suffix})");
            suffix += 1;
        }
        candidate
    };
    let category_column = derived_name("Category");
    let location_column = derived_name("Location");
    let item_column = derived_name("Item");
    let raw_columns: HashMap<_, _> = original_fields
        .into_iter()
        .map(|name| {
            let column = derived_name(&format!("$raw/{}", escaped_column(&name)));
            (name, column)
        })
        .collect();
    progress.begin("parse", groups.len() as u64, "records");
    let mut rows = Rows::new(&[
        "Name",
        &category_column,
        &location_column,
        "ImagePath",
        "Description",
        "Publisher",
        "TimeStamp",
        "AdditionalPath",
        "KeyOrValueName",
        &item_column,
        "Flags",
    ])?;
    let mut category = String::new();
    let mut location = String::new();
    let mut binary_values = false;
    for (number, ((_, id), streams)) in groups.into_iter().enumerate() {
        progress.check()?;
        let mut fields = Vec::new();
        let mut flags = 0u32;
        let mut name = String::new();
        for (key, path, length) in streams {
            progress.check()?;
            ensure!(length <= 64 * 1024 * 1024, "ARN stream exceeds 64 MiB");
            let mut data = Vec::new();
            compound.open_stream(&path)?.read_to_end(&mut data)?;
            let value = if key.eq_ignore_ascii_case("Flags") && data.len() == 4 {
                flags = u32::from_le_bytes(data[..4].try_into().unwrap());
                format!("0x{flags:08X}")
            } else if key.eq_ignore_ascii_case("TimeStamp") && data.len() == 8 {
                filetime(u64::from_le_bytes(data[..8].try_into().unwrap()))
            } else if let Some(text) = arn_string(&data) {
                text
            } else {
                binary_values = true;
                let raw = hex_bytes(&data);
                fields.push((raw_columns[&key].clone(), raw.clone()));
                crate::binary::readable(&data).unwrap_or(raw)
            };
            if key.eq_ignore_ascii_case("Name") {
                name = value.clone();
            }
            fields.push((key, value));
        }
        if flags & 0x1000 != 0 {
            location = name;
        } else if flags & 0x100 != 0 {
            category = name;
            location.clear();
        }
        fields.extend([
            (category_column.clone(), category.clone()),
            (location_column.clone(), location.clone()),
            (item_column.clone(), id),
        ]);
        rows.fields(fields)?;
        progress.done.store(number as u64 + 1, Ordering::Relaxed);
        progress.records.store(number as u64 + 1, Ordering::Relaxed);
    }
    if binary_values {
        rows.warnings.push("Unknown Autoruns binary fields show probable readable strings when present; $raw columns retain their original hexadecimal bytes.".into());
    }
    rows.finish()
}
fn arn_string(data: &[u8]) -> Option<String> {
    let prefix = data.get(..4)?;
    let length = u32::from_le_bytes(prefix.try_into().ok()?) as usize;
    let end = 4usize.checked_add(length.checked_mul(2)?)?;
    if end != data.len() {
        return None;
    }
    String::from_utf16(
        &data[4..end]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect::<Vec<_>>(),
    )
    .ok()
    .map(|s| s.trim_end_matches('\0').to_owned())
}
fn filetime(ticks: u64) -> String {
    if ticks == 0 {
        return String::new();
    }
    let seconds = (i128::from(ticks) / 10_000_000 - 11_644_473_600) as i64;
    chrono::DateTime::from_timestamp(seconds, ((ticks % 10_000_000) * 100) as u32)
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| ticks.to_string())
}

#[cfg(test)]
mod tests {
    use super::ese_text;

    #[test]
    fn portable_ese_values_use_guid_byte_order_and_ole_negative_date_semantics() {
        assert_eq!(
            ese_text(ese_core::EseValue::Guid([
                0x33, 0x22, 0x11, 0x00, 0x55, 0x44, 0x77, 0x66, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ])),
            "00112233-4455-6677-8899-aabbccddeeff"
        );
        assert_eq!(
            ese_text(ese_core::EseValue::DateTime(-1.25)),
            "1899-12-29T06:00:00+00:00"
        );
        assert_eq!(ese_text(ese_core::EseValue::DateTime(f64::NAN)), "NaN");
    }
}
