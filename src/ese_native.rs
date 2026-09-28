//! Native ESENT resolves long/multivalued columns in a clean, read-only database.
use crate::{
    data::{Progress, open_read},
    formats::{Normalized, Rows},
};
use anyhow::{Context, Result, ensure};
use std::{
    io::Read,
    path::Path,
    ptr::{null, null_mut},
    sync::atomic::{AtomicU64, Ordering},
};
use windows_sys::Win32::Storage::{Jet::*, StructuredStorage::JET_TABLEID};
fn wide(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
#[track_caller]
fn check(code: i32) -> Result<()> {
    if code < 0 {
        let mut error = code as usize;
        let mut description = [0u16; 512];
        unsafe {
            JetGetSystemParameterW(
                0,
                0,
                JET_paramErrorToString,
                &mut error,
                description.as_mut_ptr(),
                (description.len() * 2) as u32,
            );
        }
        let length = description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(description.len());
        anyhow::bail!(
            "ESENT error {code}: {}",
            String::from_utf16_lossy(&description[..length])
        );
    }
    Ok(())
}
struct Session {
    instance: JET_INSTANCE,
    id: JET_SESID,
    _temp: tempfile::TempDir,
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            if self.id != 0 {
                JetEndSession(self.id, 0);
            }
            if self.instance != 0 {
                JetTerm2(self.instance, JET_bitTermAbrupt);
            }
        }
    }
}
struct Table {
    session: JET_SESID,
    id: JET_TABLEID,
}
impl Drop for Table {
    fn drop(&mut self) {
        unsafe {
            if self.id != 0 {
                JetCloseTable(self.session, self.id);
            }
        }
    }
}
impl Table {
    fn move_to(&self, n: i32) -> Result<bool> {
        let code = unsafe { JetMove(self.session, self.id, n, 0) };
        if code == JET_errNoCurrentRecord {
            return Ok(false);
        }
        check(code)?;
        Ok(true)
    }
    fn value(&self, column: u32, tag: u32) -> Result<Option<Vec<u8>>> {
        let mut info = JET_RETRIEVECOLUMN {
            columnid: column,
            itagSequence: tag,
            grbit: JET_bitRetrieveNull,
            ..Default::default()
        };
        let mut bytes = vec![0; 4096];
        loop {
            info.pvData = bytes.as_mut_ptr().cast();
            info.cbData = bytes.len() as u32;
            check(unsafe { JetRetrieveColumns(self.session, self.id, &mut info, 1) })?;
            if info.err == JET_wrnColumnNull as i32 {
                return Ok(None);
            }
            check(info.err)?;
            let size = info.cbActual as usize;
            if size <= bytes.len() {
                ensure!(
                    info.err != JET_wrnBufferTruncated as i32,
                    "ESENT returned a truncated column without its full length"
                );
                bytes.truncate(size);
                return Ok(Some(bytes));
            }
            bytes
                .try_reserve_exact(size - bytes.len())
                .context("Not enough memory to read the complete ESE column")?;
            bytes.resize(size, 0);
        }
    }
    fn value_count(&self, column: u32) -> Result<u32> {
        let mut info = JET_RETRIEVECOLUMN {
            columnid: column,
            itagSequence: 0,
            grbit: JET_bitRetrieveNull,
            ..Default::default()
        };
        check(unsafe { JetRetrieveColumns(self.session, self.id, &mut info, 1) })?;
        check(info.err)?;
        Ok(info.itagSequence)
    }
    fn number(&self, column: u32) -> Result<u32> {
        let b = self.value(column, 1)?.unwrap_or_default();
        let mut v = [0; 4];
        let n = b.len().min(4);
        v[..n].copy_from_slice(&b[..n]);
        Ok(u32::from_le_bytes(v))
    }
    fn name(&self, column: u32) -> Result<String> {
        // ESENT catalog identifiers are ANSI even with the W API entry points.
        Ok(encoding_rs::WINDOWS_1252
            .decode(&self.value(column, 1)?.unwrap_or_default())
            .0
            .trim_end_matches('\0')
            .into())
    }
}
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static ENGINE: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub fn read(path: &Path, progress: &Progress) -> Result<Normalized> {
    // ESENT's database page size is process-global, including active instances.
    let _engine = loop {
        progress.check()?;
        match ENGINE.try_lock() {
            Ok(lock) => break lock,
            Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let mut header = [0; 240];
    open_read(path)?.read_exact(&mut header)?;
    let page_size = u32::from_le_bytes(header[236..240].try_into().unwrap());
    let page_size = if page_size == 0 { 4096 } else { page_size };
    ensure!(
        matches!(page_size, 2048 | 4096 | 8192 | 16384 | 32768),
        "Unsupported ESE database page size {page_size}"
    );
    let mut session = Session {
        instance: Default::default(),
        id: Default::default(),
        _temp: tempfile::tempdir()?,
    };
    let name = wide(format!(
        "Timeline-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    unsafe {
        check(JetSetSystemParameterW(
            null_mut(),
            0,
            JET_paramDatabasePageSize,
            page_size as usize,
            null(),
        ))?;
        check(JetCreateInstanceW(&mut session.instance, name.as_ptr()))?;

        check(JetSetSystemParameterW(
            &mut session.instance,
            0,
            JET_paramDisableCallbacks,
            1,
            null(),
        ))?;

        check(JetSetSystemParameterW(
            &mut session.instance,
            session.id,
            JET_paramRecovery,
            0,
            wide("off").as_ptr(),
        ))?;
        for param in [JET_paramSystemPath, JET_paramLogFilePath, JET_paramTempPath] {
            check(JetSetSystemParameterW(
                &mut session.instance,
                session.id,
                param,
                0,
                wide(if param == JET_paramTempPath {
                    session._temp.path().join("temp.edb")
                } else {
                    session._temp.path().to_owned()
                })
                .as_ptr(),
            ))?;
        }
        check(JetInit(&mut session.instance))?;
        check(JetBeginSessionW(
            session.instance,
            &mut session.id,
            null(),
            null(),
        ))?;
        check(JetAttachDatabaseW(
            session.id,
            wide(path).as_ptr(),
            JET_bitDbReadOnly,
        ))?;
    }
    let mut database = 0;
    check(unsafe {
        JetOpenDatabaseW(
            session.id,
            wide(path).as_ptr(),
            null(),
            &mut database,
            JET_bitDbReadOnly,
        )
    })?;
    let mut objects = JET_OBJECTLIST {
        cbStruct: std::mem::size_of::<JET_OBJECTLIST>() as u32,
        ..Default::default()
    };
    check(unsafe {
        JetGetObjectInfoA(
            session.id,
            database,
            1,
            null(),
            null(),
            (&mut objects as *mut JET_OBJECTLIST).cast(),
            objects.cbStruct,
            1,
        )
    })?;
    let list = Table {
        session: session.id,
        id: objects.tableid,
    };
    let mut names = Vec::new();
    let mut available = list.move_to(i32::MIN)?;
    while available {
        progress.check()?;
        names.push(list.name(objects.columnidobjectname)?);
        available = list.move_to(1)?;
    }
    let mut output = Rows::new(&["Table"])?;
    let mut interpreted_binary = false;
    for name in names {
        progress.check()?;
        let mut table = Table {
            session: session.id,
            id: Default::default(),
        };
        check(unsafe {
            JetOpenTableW(
                session.id,
                database,
                wide(&name).as_ptr(),
                null(),
                0,
                JET_bitTableReadOnly,
                &mut table.id,
            )
        })?;
        let mut columns = JET_COLUMNLIST {
            cbStruct: std::mem::size_of::<JET_COLUMNLIST>() as u32,
            ..Default::default()
        };
        check(unsafe {
            JetGetTableColumnInfoA(
                session.id,
                table.id,
                null(),
                (&mut columns as *mut JET_COLUMNLIST).cast(),
                columns.cbStruct,
                1,
            )
        })?;
        let column_table = Table {
            session: session.id,
            id: columns.tableid,
        };
        let mut schema = Vec::new();
        let mut available = column_table.move_to(i32::MIN)?;
        while available {
            progress.check()?;
            schema.push((
                column_table.name(columns.columnidcolumnname)?,
                column_table.number(columns.columnidcolumnid)?,
                column_table.number(columns.columnidcoltyp)?,
                column_table.number(columns.columnidCp)?,
                column_table.number(columns.columnidgrbit)?,
            ));
            available = column_table.move_to(1)?;
        }
        let mut available = table.move_to(i32::MIN)?;
        while available {
            progress.check()?;
            let mut fields = vec![("Table".into(), name.clone())];
            for (column, id, kind, cp, flags) in &schema {
                let mut values = Vec::new();
                let mut original = Vec::new();
                let multi = flags & JET_bitColumnMultiValued != 0;
                let count = if multi { table.value_count(*id)? } else { 1 };
                for tag in 1..=count {
                    progress.check()?;
                    let bytes = table
                        .value(*id, tag)
                        .with_context(|| format!("Table {name}, column {column}, value {tag}"))?;
                    let (text, raw) = bytes
                        .map(|bytes| {
                            let (text, raw) = decode_field(*kind, *cp, &bytes);
                            (Some(text), raw)
                        })
                        .unwrap_or_default();
                    values.push(text);
                    original.push(raw);
                }
                let column = column.replace('~', "~0").replace('/', "~1");
                fields.push((
                    format!("/{column}"),
                    if multi {
                        serde_json::to_string(&values)?
                    } else if values.len() == 1 {
                        values.pop().flatten().unwrap_or_default()
                    } else if values.is_empty() {
                        String::new()
                    } else {
                        serde_json::to_string(&values)?
                    },
                ));
                if original.iter().any(Option::is_some) {
                    interpreted_binary = true;
                    fields.push((
                        format!("$raw/{column}"),
                        if multi {
                            serde_json::to_string(&original)?
                        } else {
                            original.pop().flatten().unwrap_or_default()
                        },
                    ));
                }
            }
            output.fields(fields)?;
            progress.records.fetch_add(1, Ordering::Relaxed);
            available = table.move_to(1)?;
        }
    }
    let mut result = output.finish()?;
    if interpreted_binary {
        result.warnings.push("Binary values show probable readable strings when present; $raw columns retain their original hexadecimal bytes.".into());
    }
    Ok(result)
}
fn typed_value(kind: u32, cp: u32, b: &[u8]) -> Option<String> {
    macro_rules! number {
        ($t:ty) => {
            if b.len() == std::mem::size_of::<$t>() {
                return Some(<$t>::from_le_bytes(b.try_into().unwrap()).to_string());
            }
        };
    }
    match kind {
        1 if b.len() == 1 => return Some((b[0] != 0).to_string()),
        2 => {
            number!(u8)
        }
        3 => {
            number!(i16)
        }
        4 => {
            number!(i32)
        }
        5 | 15 => {
            number!(i64)
        }
        6 => {
            number!(f32)
        }
        7 => {
            number!(f64)
        }
        14 => {
            number!(u32)
        }
        17 => {
            number!(u16)
        }
        18 => {
            number!(u64)
        }
        8 if b.len() == 8 => {
            let days = f64::from_le_bytes(b.try_into().unwrap());
            // OLE DATE uses the absolute fractional part for the time of day,
            // including dates before 1899-12-30 (for example -1.25 is 06:00).
            let seconds = (days.trunc() - 25569.0) * 86400.0 + days.fract().abs() * 86400.0;
            if seconds.is_finite() && seconds.abs() < i64::MAX as f64 {
                let whole = seconds.floor();
                let nanos = ((seconds - whole) * 1_000_000_000.0).round() as u32;
                let carry = u32::from(nanos == 1_000_000_000);
                if let Some(date) = chrono::DateTime::from_timestamp(
                    whole as i64 + i64::from(carry),
                    nanos % 1_000_000_000,
                ) {
                    return Some(date.to_rfc3339());
                }
            }
            return Some(days.to_string());
        }
        10 | 12 => {
            if cp == 1200 && b.len().is_multiple_of(2) {
                if let Ok(value) = String::from_utf16(
                    &b.chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect::<Vec<_>>(),
                ) && valid_text(&value)
                {
                    return Some(value);
                }
            } else if let Some(encoding) = match cp {
                0 | 1252 => Some(encoding_rs::WINDOWS_1252),
                65001 => Some(encoding_rs::UTF_8),
                1250 => Some(encoding_rs::WINDOWS_1250),
                1251 => Some(encoding_rs::WINDOWS_1251),
                1253 => Some(encoding_rs::WINDOWS_1253),
                1254 => Some(encoding_rs::WINDOWS_1254),
                1255 => Some(encoding_rs::WINDOWS_1255),
                1256 => Some(encoding_rs::WINDOWS_1256),
                1257 => Some(encoding_rs::WINDOWS_1257),
                1258 => Some(encoding_rs::WINDOWS_1258),
                932 => Some(encoding_rs::SHIFT_JIS),
                936 => Some(encoding_rs::GBK),
                949 => Some(encoding_rs::EUC_KR),
                950 => Some(encoding_rs::BIG5),
                _ => None,
            } {
                let (value, errors) = encoding.decode_without_bom_handling(b);
                if !errors && valid_text(&value) {
                    return Some(value.into_owned());
                }
            }
            // Preserve every original byte when the declared encoding is
            // unknown or invalid, rather than silently substituting U+FFFD.
        }
        16 if b.len() == 16 => {
            return Some(format!(
                "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                u32::from_le_bytes(b[..4].try_into().unwrap()),
                u16::from_le_bytes(b[4..6].try_into().unwrap()),
                u16::from_le_bytes(b[6..8].try_into().unwrap()),
                b[8],
                b[9],
                b[10],
                b[11],
                b[12],
                b[13],
                b[14],
                b[15]
            ));
        }
        _ => {}
    }
    None
}

fn valid_text(text: &str) -> bool {
    !text
        .chars()
        .any(|c| c == '\u{fffd}' || c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}

fn decode_field(kind: u32, cp: u32, bytes: &[u8]) -> (String, Option<String>) {
    if let Some(text) = typed_value(kind, cp, bytes) {
        return (text, None);
    }
    let raw = crate::binary::hex(bytes);
    match crate::binary::readable(bytes) {
        Some(text) => (text, Some(raw)),
        None => (raw, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn decode(kind: u32, cp: u32, bytes: &[u8]) -> String {
        decode_field(kind, cp, bytes).0
    }
    #[test]
    fn native_esent_reads_real_database_without_modifying_it() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("evidence.edb");
        let expected = "Unicode пример ".repeat(10000);
        let binary: Vec<u8> = include_str!("../tests/fixtures/binary-url.hex")
            .split_whitespace()
            .map(|byte| u8::from_str_radix(byte, 16).unwrap())
            .collect();
        let expected_url = String::from_utf16(
            &binary[16..binary.len() - 4]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        )?;
        {
            let _engine = ENGINE.lock().unwrap_or_else(|error| error.into_inner());
            let mut session = Session {
                instance: 0,
                id: 0,
                _temp: tempfile::tempdir()?,
            };
            unsafe {
                check(JetCreateInstanceW(
                    &mut session.instance,
                    wide("Timeline-fixture").as_ptr(),
                ))?;

                check(JetSetSystemParameterW(
                    &mut session.instance,
                    0,
                    JET_paramRecovery,
                    0,
                    wide("off").as_ptr(),
                ))?;
                for param in [JET_paramSystemPath, JET_paramLogFilePath, JET_paramTempPath] {
                    check(JetSetSystemParameterW(
                        &mut session.instance,
                        0,
                        param,
                        0,
                        wide(session._temp.path()).as_ptr(),
                    ))?;
                }
                check(JetInit(&mut session.instance))?;
                check(JetBeginSessionW(
                    session.instance,
                    &mut session.id,
                    null(),
                    null(),
                ))?;
                let mut database = 0;
                check(JetCreateDatabaseW(
                    session.id,
                    wide(&path).as_ptr(),
                    null(),
                    &mut database,
                    0,
                ))?;
                let mut table = Table {
                    session: session.id,
                    id: 0,
                };
                check(JetCreateTableW(
                    session.id,
                    database,
                    wide("Entrées").as_ptr(),
                    0,
                    100,
                    &mut table.id,
                ))?;
                let definition = JET_COLUMNDEF {
                    cbStruct: std::mem::size_of::<JET_COLUMNDEF>() as u32,
                    coltyp: 12,
                    cp: 1200,
                    ..Default::default()
                };
                let mut column = 0;
                check(JetAddColumnW(
                    session.id,
                    table.id,
                    wide("Détails").as_ptr(),
                    &definition,
                    null(),
                    0,
                    &mut column,
                ))?;
                let multi_definition = JET_COLUMNDEF {
                    grbit: JET_bitColumnMultiValued | JET_bitColumnTagged,
                    ..definition
                };
                let mut multi_column = 0;
                check(JetAddColumnW(
                    session.id,
                    table.id,
                    wide("Values").as_ptr(),
                    &multi_definition,
                    null(),
                    0,
                    &mut multi_column,
                ))?;
                let binary_definition = JET_COLUMNDEF {
                    coltyp: 11,
                    cp: 0,
                    ..definition
                };
                let mut binary_column = 0;
                check(JetAddColumnW(
                    session.id,
                    table.id,
                    wide("Payload").as_ptr(),
                    &binary_definition,
                    null(),
                    0,
                    &mut binary_column,
                ))?;
                check(JetBeginTransaction(session.id))?;
                check(JetPrepareUpdate(session.id, table.id, 0))?;
                let bytes: Vec<u8> = expected.encode_utf16().flat_map(u16::to_le_bytes).collect();
                check(JetSetColumn(
                    session.id,
                    table.id,
                    column,
                    bytes.as_ptr().cast(),
                    bytes.len() as u32,
                    0,
                    null(),
                ))?;
                for value in ["first", "", "последний"] {
                    let bytes: Vec<u8> = value.encode_utf16().flat_map(u16::to_le_bytes).collect();
                    let info = JET_SETINFO {
                        cbStruct: std::mem::size_of::<JET_SETINFO>() as u32,
                        itagSequence: 0,
                        ..Default::default()
                    };
                    check(JetSetColumn(
                        session.id,
                        table.id,
                        multi_column,
                        bytes.as_ptr().cast(),
                        bytes.len() as u32,
                        if bytes.is_empty() {
                            JET_bitSetZeroLength
                        } else {
                            0
                        },
                        &info,
                    ))?;
                }
                check(JetSetColumn(
                    session.id,
                    table.id,
                    binary_column,
                    binary.as_ptr().cast(),
                    binary.len() as u32,
                    0,
                    null(),
                ))?;
                check(JetUpdate(session.id, table.id, null_mut(), 0, null_mut()))?;
                check(JetCommitTransaction(session.id, 0))?;
                drop(table);
                check(JetCloseDatabase(session.id, database, 0))?;
                check(JetDetachDatabaseW(session.id, wide(&path).as_ptr()))?;
                check(JetEndSession(session.id, 0))?;
                session.id = 0;
                check(JetTerm(session.instance))?;
                session.instance = 0;
            }
        }
        let before = std::fs::read(&path)?;
        let normalized = read(&path, &Progress::default())?;
        let mut csv = csv::ReaderBuilder::new()
            .has_headers(false)
            .from_path(normalized.file.path())?;
        let record = csv.records().next().unwrap()?;
        assert_eq!(&record[0], "Entrées");
        let details = normalized
            .headers
            .iter()
            .position(|name| name == "/Détails")
            .unwrap();
        let values = normalized
            .headers
            .iter()
            .position(|name| name == "/Values")
            .unwrap();
        assert_eq!(&record[details], expected);
        assert_eq!(&record[values], "[\"first\",\"\",\"последний\"]");
        let payload = normalized
            .headers
            .iter()
            .position(|name| name == "/Payload")
            .unwrap();
        let raw = normalized
            .headers
            .iter()
            .position(|name| name == "$raw/Payload")
            .unwrap();
        assert_eq!(&record[payload], expected_url);
        assert_eq!(&record[raw], crate::binary::hex(&binary));
        assert_eq!(std::fs::read(&path)?, before);
        Ok(())
    }

    #[test]
    fn ese_decoding_preserves_malformed_text_and_scalar_values() {
        assert_eq!(decode(12, 1200, &[b'A', 0, 0xff]), "41 00 FF");
        assert_eq!(decode(12, 1200, &[0, 0xd8]), "00 D8");
        assert_eq!(decode(12, 65001, "пример".as_bytes()), "пример");
        assert_eq!(decode(10, 1252, &[b'c', b'a', b'f', 0xe9]), "café");
        assert_eq!(decode(10, 42, &[0xff]), "FF");
        let (value, raw) = decode_field(10, 1252, &[0xff, 0xfe, b'A', 0]);
        assert_eq!(raw.as_deref(), Some("FF FE 41 00"));
        assert_eq!(value, "A");
        assert_eq!(decode(18, 0, &u64::MAX.to_le_bytes()), u64::MAX.to_string());
        assert_eq!(
            decode(8, 0, &(-1.25f64).to_le_bytes()),
            "1899-12-29T06:00:00+00:00"
        );
        assert_eq!(decode(8, 0, &f64::NAN.to_le_bytes()), "NaN");
        assert_eq!(
            decode(
                16,
                0,
                &[
                    0x78, 0x56, 0x34, 0x12, 0x34, 0x12, 0xcd, 0xab, 1, 2, 3, 4, 5, 6, 7, 8
                ]
            ),
            "12345678-1234-abcd-0102-030405060708"
        );
    }
}
