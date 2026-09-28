//! Acquisition is read-only. A changing/locked source is always copied before indexing.
use crate::data::{Progress, open_read};
use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};
use tempfile::{NamedTempFile, TempPath};

pub struct Acquired {
    pub path: PathBuf,
    pub temporary: Vec<TempPath>,
    pub method: &'static str,
    // Keep the successful non-writing handle alive until Dataset has its own
    // handle. Otherwise a writer can slip in between the probe and indexing.
    _source: Option<File>,
}
#[derive(Debug)]
pub struct NeedsElevation;
impl std::fmt::Display for NeedsElevation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Для чтения этого файла нужны права администратора")
    }
}
impl std::error::Error for NeedsElevation {}

pub fn acquire(path: &Path, progress: &Progress) -> Result<Acquired> {
    progress.check()?;
    if let Ok(source) = open_read(path) {
        return Ok(Acquired {
            path: path.to_owned(),
            temporary: vec![],
            method: "direct",
            _source: Some(source),
        });
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(7).custom_flags(0x02000000);
    }
    let error = match options.open(path) {
        Ok(mut input) => {
            let before = input.metadata()?;
            ensure!(before.is_file(), "Source is not a regular file");
            let length = before.len();
            let mut temp = NamedTempFile::new()?;
            progress.total.store(length, Ordering::Relaxed);
            let mut remaining = length;
            let mut buffer = vec![0; 1024 * 1024];
            while remaining > 0 {
                progress.check()?;
                let capacity = remaining.min(buffer.len() as u64) as usize;
                let n = input.read(&mut buffer[..capacity])?;
                ensure!(
                    n > 0,
                    "Source changed while taking a snapshot; reopen the file"
                );
                temp.write_all(&buffer[..n])?;
                remaining -= n as u64;
                progress.done.store(length - remaining, Ordering::Relaxed);
            }
            let after = input.metadata()?;
            ensure!(
                before.len() == after.len() && before.modified().ok() == after.modified().ok(),
                "Source changed while taking a snapshot; reopen the file"
            );
            return Ok(temporary(temp, "shared-snapshot"));
        }
        Err(error) => error,
    };
    #[cfg(windows)]
    {
        if path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("evtx"))
            && let Ok(temp) = export_event_log(path)
        {
            return Ok(temporary(temp, "event-log-api"));
        }
        let usn = is_live_usn_journal(path);
        if matches!(error.raw_os_error(), Some(5 | 32 | 33)) || usn {
            if !is_elevated() {
                return Err(NeedsElevation.into());
            }
            if usn {
                return Ok(temporary(snapshot_usn(path, progress)?, "usn-api"));
            }
            return Ok(temporary(copy_ntfs(path, progress)?, "ntfs-snapshot"));
        }
    }
    Err(error).with_context(|| format!("Cannot acquire {} for reading", path.display()))
}
fn temporary(temp: NamedTempFile, method: &'static str) -> Acquired {
    Acquired {
        path: temp.path().to_owned(),
        temporary: vec![temp.into_temp_path()],
        method,
        _source: None,
    }
}

#[cfg(windows)]
fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
#[cfg(windows)]
#[link(name = "shell32")]
unsafe extern "system" {
    fn IsUserAnAdmin() -> i32;
    fn ShellExecuteW(
        window: *mut std::ffi::c_void,
        verb: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> isize;
}
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        unsafe { IsUserAnAdmin() != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}
pub fn relaunch_elevated(path: &Path, settings: &str) -> Result<()> {
    #[cfg(windows)]
    {
        let executable = wide(std::env::current_exe()?);
        let args = wide(format!(
            "{} {}",
            quote_argument(&path.to_string_lossy()),
            quote_argument(settings)
        ));
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                wide("runas").as_ptr(),
                executable.as_ptr(),
                args.as_ptr(),
                std::ptr::null(),
                1,
            )
        };
        ensure!(
            result > 32,
            "UAC cancelled or Windows could not start the elevated window ({result})"
        );
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (path, settings);
        anyhow::bail!("Elevation is available on Windows only")
    }
}
fn quote_argument(value: &str) -> String {
    let mut output = String::from("\"");
    let mut slashes = 0;
    for c in value.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '"' {
            output.push_str(&"\\".repeat(slashes * 2 + 1));
        } else {
            output.push_str(&"\\".repeat(slashes));
        }
        slashes = 0;
        output.push(c);
    }
    output.push_str(&"\\".repeat(slashes * 2));
    output.push('"');
    output
}
#[cfg(windows)]
fn export_event_log(path: &Path) -> Result<NamedTempFile> {
    #[link(name = "wevtapi")]
    unsafe extern "system" {
        fn EvtExportLog(
            session: isize,
            path: *const u16,
            query: *const u16,
            target: *const u16,
            flags: u32,
        ) -> i32;
    }
    let directory = tempfile::tempdir()?;
    let target = directory.path().join("snapshot.evtx");
    let ok = unsafe {
        EvtExportLog(
            0,
            wide(path).as_ptr(),
            std::ptr::null(),
            wide(&target).as_ptr(),
            2,
        )
    };
    ensure!(
        ok != 0,
        "Event log snapshot: {}",
        std::io::Error::last_os_error()
    );
    let mut output = NamedTempFile::new()?;
    std::io::copy(&mut File::open(&target)?, &mut output)?;
    Ok(output)
}

#[cfg(windows)]
fn local_target(path: &Path) -> Result<(String, Vec<String>, String)> {
    let value = path
        .to_str()
        .context("Raw NTFS path is not valid Unicode")?;
    let value = value.strip_prefix(r"\\?\").unwrap_or(value);
    let bytes = value.as_bytes();
    ensure!(
        bytes.len() > 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/'),
        "Raw NTFS access needs an absolute path on a local NTFS drive"
    );
    let mut parts: Vec<String> = value[3..]
        .split(['\\', '/'])
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    ensure!(
        !parts.is_empty() && !parts.iter().any(|p| p == ".." || p == "."),
        "Invalid NTFS path"
    );
    ensure!(
        !parts[..parts.len() - 1]
            .iter()
            .any(|part| part.contains(':')),
        "Invalid NTFS path component"
    );
    let last = parts.last_mut().unwrap();
    let fields: Vec<_> = last.split(':').collect();
    ensure!(
        !fields[0].is_empty() && fields.len() <= 3,
        "Invalid NTFS stream path"
    );
    if fields.len() == 3 {
        ensure!(
            fields[2].eq_ignore_ascii_case("$DATA"),
            "Only NTFS $DATA streams can be read"
        );
    }
    let stream = fields.get(1).copied().unwrap_or("").to_owned();
    *last = fields[0].to_owned();
    Ok((format!(r"\\.\{}:", bytes[0] as char), parts, stream))
}
/// Recognize the journal stream without confusing it with $Max metadata or
/// ordinary filenames containing "$UsnJrnl".
pub fn is_usn_journal_path(path: &Path) -> bool {
    let Some(value) = path.to_str() else {
        return false;
    };
    let name = value.rsplit(['\\', '/']).next().unwrap_or("");
    let fields: Vec<_> = name.split(':').collect();
    fields[0].eq_ignore_ascii_case("$UsnJrnl")
        && (fields.len() == 1
            || (matches!(fields.len(), 2 | 3)
                && fields[1].eq_ignore_ascii_case("$J")
                && (fields.len() == 2 || fields[2].eq_ignore_ascii_case("$DATA"))))
}
#[cfg(windows)]
fn is_live_usn_journal(path: &Path) -> bool {
    is_usn_journal_path(path)
        && local_target(path)
            .is_ok_and(|(_, parts, _)| parts.len() == 2 && parts[0].eq_ignore_ascii_case("$Extend"))
}
#[cfg(windows)]
fn volume(path: &str) -> Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    Ok(OpenOptions::new().read(true).share_mode(7).open(path)?)
}
#[cfg(windows)]
fn copy_ntfs(path: &Path, progress: &Progress) -> Result<NamedTempFile> {
    use ntfs::{Ntfs, NtfsAttributeFlags, NtfsReadSeek, indexes::NtfsFileNameIndex};
    let (device, parts, stream) = local_target(path)?;
    let mut fs = BufReader::with_capacity(
        1024 * 1024,
        AlignedReader {
            file: volume(&device)?,
            position: 0,
            scratch: Vec::new(),
        },
    );
    let mut ntfs = Ntfs::new(&mut fs)?;
    ntfs.read_upcase_table(&mut fs)?;
    let mut current = ntfs.root_directory(&mut fs)?;
    for part in parts {
        progress.check()?;
        let index = current.directory_index(&mut fs)?;
        let mut finder = index.finder();
        let entry = NtfsFileNameIndex::find(&mut finder, &ntfs, &mut fs, &part)
            .context("NTFS path component not found")??;
        current = entry.to_file(&ntfs, &mut fs)?;
    }
    let item = current
        .data(&mut fs, &stream)
        .context("NTFS data stream not found")??;
    let attr = item.to_attribute()?;
    ensure!(
        !attr
            .flags()
            .intersects(NtfsAttributeFlags::COMPRESSED | NtfsAttributeFlags::ENCRYPTED),
        "Raw NTFS acquisition cannot decode compressed or encrypted streams; export a readable copy first"
    );
    let mut data = attr.value(&mut fs)?;
    let expected = data.len();
    progress.total.store(expected, Ordering::Relaxed);
    let mut temp = NamedTempFile::new()?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut total = 0;
    loop {
        progress.check()?;
        let size = data.read(&mut fs, &mut buffer)?;
        if size == 0 {
            break;
        }
        temp.write_all(&buffer[..size])?;
        total += size as u64;
        progress.done.store(total, Ordering::Relaxed);
    }
    ensure!(
        total == expected,
        "Incomplete NTFS snapshot: read {total} of {expected} bytes"
    );
    Ok(temp)
}
#[cfg(windows)]
struct AlignedReader {
    file: File,
    position: u64,
    scratch: Vec<u8>,
}
#[cfg(windows)]
impl Read for AlignedReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let start = self.position / 4096 * 4096;
        let delta = (self.position - start) as usize;
        let length = delta
            .checked_add(buffer.len())
            .and_then(|n| n.checked_add(4095))
            .map(|n| n / 4096 * 4096)
            .ok_or_else(|| std::io::Error::other("Volume read size overflow"))?;
        self.scratch.resize(length, 0);
        self.file.seek(SeekFrom::Start(start))?;
        let size = self.file.read(&mut self.scratch)?;
        let size = size.saturating_sub(delta).min(buffer.len());
        buffer[..size].copy_from_slice(&self.scratch[delta..delta + size]);
        self.position = self
            .position
            .checked_add(size as u64)
            .ok_or_else(|| std::io::Error::other("Volume position overflow"))?;
        Ok(size)
    }
}
#[cfg(windows)]
impl Seek for AlignedReader {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.position = match from {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(n) => self
                .position
                .checked_add_signed(n)
                .context("Invalid volume offset")
                .map_err(std::io::Error::other)?,
            SeekFrom::End(_) => {
                return Err(std::io::Error::other(
                    "End-relative volume seek is unsupported",
                ));
            }
        };
        Ok(self.position)
    }
}
#[cfg(windows)]
fn snapshot_usn(path: &Path, progress: &Progress) -> Result<NamedTempFile> {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn DeviceIoControl(
            handle: *mut std::ffi::c_void,
            code: u32,
            input: *const u8,
            in_size: u32,
            output: *mut u8,
            out_size: u32,
            returned: *mut u32,
            overlapped: *mut std::ffi::c_void,
        ) -> i32;
    }
    let (device, _, _) = local_target(path)?;
    let file = volume(&device)?;
    let ioctl = |code, input: &[u8], output: &mut [u8]| -> std::io::Result<usize> {
        let mut count = 0;
        let ok = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                code,
                input.as_ptr(),
                input.len() as u32,
                output.as_mut_ptr(),
                output.len() as u32,
                &mut count,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        if count as usize > output.len() {
            return Err(std::io::Error::other("Invalid USN response size"));
        }
        Ok(count as usize)
    };
    let mut journal = [0; 80];
    let count = ioctl(0x000900f4, &[], &mut journal)?;
    ensure!(count >= 56, "Invalid USN journal metadata");
    let u64_at = |start| u64::from_le_bytes(journal[start..start + 8].try_into().unwrap());
    let id = u64_at(0);
    let mut next = u64_at(8).max(u64_at(24));
    let end = u64_at(16);
    ensure!(
        next <= end && end <= i64::MAX as u64,
        "Invalid USN journal range"
    );
    let (min_version, max_version) = if count >= 60 {
        let min = u16::from_le_bytes(journal[56..58].try_into().unwrap()).max(2);
        let max = u16::from_le_bytes(journal[58..60].try_into().unwrap()).min(4);
        ensure!(
            min <= max,
            "USN volume has no supported record version (2, 3, 4)"
        );
        (min, max)
    } else {
        (2, 3)
    };
    progress
        .total
        .store(end.saturating_sub(next), Ordering::Relaxed);
    let first = next;
    let mut temp = NamedTempFile::new()?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut legacy = false;
    while next < end {
        progress.check()?;
        let mut request = [0; 48];
        request[..8].copy_from_slice(&next.to_le_bytes());
        request[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        request[32..40].copy_from_slice(&id.to_le_bytes());
        request[40..42].copy_from_slice(&min_version.to_le_bytes());
        request[42..44].copy_from_slice(&max_version.to_le_bytes());
        let count = match ioctl(
            0x000900bb,
            &request[..if legacy { 40 } else { 48 }],
            &mut buffer,
        ) {
            Ok(count) => count,
            Err(error)
                if !legacy
                    && min_version == 2
                    && matches!(error.raw_os_error(), Some(1 | 50 | 87)) =>
            {
                legacy = true;
                continue;
            }
            Err(error) => return Err(error).context("USN journal snapshot is incomplete"),
        };
        ensure!(count >= 8, "Invalid USN response");
        let following = u64::from_le_bytes(buffer[..8].try_into().unwrap());
        ensure!(
            following > next,
            "USN journal stopped before the captured end; reopen the file"
        );
        write_usn_records(&mut temp, &buffer[8..count], end)?;
        next = following;
        progress
            .done
            .store(next.min(end).saturating_sub(first), Ordering::Relaxed);
    }
    let count = ioctl(0x000900f4, &[], &mut journal)?;
    ensure!(
        count >= 56 && u64::from_le_bytes(journal[..8].try_into().unwrap()) == id,
        "USN journal was replaced during acquisition; reopen the file"
    );
    Ok(temp)
}

#[cfg(windows)]
fn write_usn_records(output: &mut impl Write, mut bytes: &[u8], end: u64) -> Result<()> {
    while !bytes.is_empty() {
        ensure!(bytes.len() >= 8, "Truncated USN record header");
        let length = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        let (minimum, usn_at) = match version {
            2 => (60, 24),
            3 => (76, 40),
            4 => (64, 40),
            _ => anyhow::bail!("Unsupported USN API record version {version}"),
        };
        ensure!(
            length >= minimum && length.is_multiple_of(8) && length <= bytes.len(),
            "Invalid USN record length"
        );
        let usn = u64::from_le_bytes(bytes[usn_at..usn_at + 8].try_into().unwrap());
        if usn < end {
            output.write_all(&bytes[..length])?;
        }
        bytes = &bytes[length..];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_path_does_not_match_max_or_ordinary_names() {
        for path in [
            r"C:\$Extend\$UsnJrnl:$J",
            r"\\?\C:\$Extend\$usnjrnl:$j:$data",
            r"D:\export\$UsnJrnl",
        ] {
            assert!(is_usn_journal_path(Path::new(path)), "{path}");
        }
        for path in [
            r"C:\$Extend\$UsnJrnl:$Max",
            r"C:\file-$UsnJrnl.csv",
            r"C:\$UsnJrnl\data.txt",
            r"C:\$UsnJrnl:$J:$INDEX_ALLOCATION",
        ] {
            assert!(!is_usn_journal_path(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn cancellation_prevents_acquisition() -> Result<()> {
        let file = NamedTempFile::new()?;
        let progress = Progress::default();
        progress.cancel();
        assert!(acquire(file.path(), &progress).is_err());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn native_target_preserves_ads_and_rejects_invalid_paths() -> Result<()> {
        let (drive, parts, stream) = local_target(Path::new(r"\\?\C:\folder\file:stream:$data"))?;
        assert_eq!(drive, r"\\.\C:");
        assert_eq!(parts, ["folder", "file"]);
        assert_eq!(stream, "stream");
        assert_eq!(local_target(Path::new(r"C:\file::$DATA"))?.2, "");
        for path in [
            r"\\server\share\file",
            r"C:file",
            r"C:\a\..\file",
            r"C:\file:x:$INDEX_ALLOCATION",
            r"C:\file:x:$DATA:extra",
            r"C:\a:x\file",
        ] {
            assert!(local_target(Path::new(path)).is_err(), "{path}");
        }
        assert!(!is_live_usn_journal(Path::new(r"C:\backup\$UsnJrnl:$J")));
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn shared_snapshot_preserves_content_and_cleans_up() -> Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("active.log");
        let mut writer = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(7)
            .open(&path)?;
        let bytes = vec![0xa5; 2 * 1024 * 1024 + 29];
        writer.write_all(&bytes)?;
        writer.flush()?;
        let progress = Progress::default();
        let acquired = acquire(&path, &progress)?;
        assert_eq!(acquired.method, "shared-snapshot");
        assert_eq!(std::fs::read(&acquired.path)?, bytes);
        assert_eq!(progress.done.load(Ordering::Relaxed), bytes.len() as u64);
        let temporary_path = acquired.path.clone();
        drop(acquired);
        assert!(!temporary_path.exists());
        assert_eq!(std::fs::read(&path)?, bytes);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn direct_acquisition_retains_read_lock() -> Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("stable.log");
        std::fs::write(&path, "data")?;
        let acquired = acquire(&path, &Progress::default())?;
        assert_eq!(acquired.method, "direct");
        assert!(
            OpenOptions::new()
                .write(true)
                .share_mode(7)
                .open(&path)
                .is_err()
        );
        drop(acquired);
        assert!(
            OpenOptions::new()
                .write(true)
                .share_mode(7)
                .open(&path)
                .is_ok()
        );
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn elevation_arguments_round_trip_with_windows_parser() -> Result<()> {
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn CommandLineToArgvW(command: *const u16, count: *mut i32) -> *mut *mut u16;
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn LocalFree(memory: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
        }
        let values = [
            r"C:\Folder with spaces\файл.evtx",
            r#"{"language":"ru","path":"C:\\A\\","value":"\"quoted\""}"#,
            "",
            r"C:\trailing\",
        ];
        let command = wide(format!(
            "timeline.exe {}",
            values.map(quote_argument).join(" ")
        ));
        let mut count = 0;
        let args = unsafe { CommandLineToArgvW(command.as_ptr(), &mut count) };
        ensure!(!args.is_null(), "CommandLineToArgvW failed");
        let decoded: Vec<_> = unsafe { std::slice::from_raw_parts(args, count as usize) }
            .iter()
            .skip(1)
            .map(|&arg| {
                let mut length = 0;
                unsafe {
                    while *arg.add(length) != 0 {
                        length += 1;
                    }
                    String::from_utf16_lossy(std::slice::from_raw_parts(arg, length))
                }
            })
            .collect();
        unsafe {
            LocalFree(args.cast());
        }
        assert_eq!(decoded, values);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn usn_snapshot_keeps_only_records_before_captured_end() -> Result<()> {
        let mut buffer = Vec::new();
        for (version, length, offset, usn) in [
            (2u16, 64u32, 24usize, 50u64),
            (3, 80, 40, 70),
            (4, 64, 40, 90),
        ] {
            let mut record = vec![0; length as usize];
            record[..4].copy_from_slice(&length.to_le_bytes());
            record[4..6].copy_from_slice(&version.to_le_bytes());
            record[offset..offset + 8].copy_from_slice(&usn.to_le_bytes());
            buffer.extend(record);
        }
        let mut output = Vec::new();
        write_usn_records(&mut output, &buffer, 90)?;
        assert_eq!(output, buffer[..144]);
        assert!(write_usn_records(&mut Vec::new(), &buffer[..63], 90).is_err());
        Ok(())
    }
}
