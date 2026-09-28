//! Region-based defaults and translations for app-owned native messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Russian,
    English,
}

impl Language {
    pub fn from_region(region: &str) -> Self {
        match region.to_ascii_uppercase().as_str() {
            "RU" | "BY" | "UA" => Self::Russian,
            _ => Self::English,
        }
    }
    pub fn from_code(code: Option<&str>) -> Self {
        if code == Some("en") {
            Self::English
        } else {
            Self::Russian
        }
    }
    pub fn text(self, russian: &'static str, english: &'static str) -> &'static str {
        match self {
            Self::Russian => russian,
            Self::English => english,
        }
    }
}

pub fn system_region() -> String {
    // Also allows deterministic integration tests without changing Windows settings.
    if let Ok(region) = std::env::var("TIMELINE_REGION") {
        return region.to_ascii_uppercase();
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetUserGeoID(class: i32) -> i32;
            fn GetGeoInfoW(
                geo: i32,
                kind: u32,
                buffer: *mut u16,
                length: i32,
                language: u16,
            ) -> i32;
            fn GetUserDefaultLocaleName(buffer: *mut u16, length: i32) -> i32;
        }
        let mut buffer = [0u16; 85];
        // Windows "Country or region" takes precedence over display language.
        let length = unsafe {
            GetGeoInfoW(
                GetUserGeoID(16),
                4,
                buffer.as_mut_ptr(),
                buffer.len() as i32,
                0,
            )
        };
        if length > 1 && length as usize <= buffer.len() {
            return String::from_utf16_lossy(&buffer[..length as usize - 1]).to_ascii_uppercase();
        }
        let length = unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), buffer.len() as i32) };
        if length > 1 && length as usize <= buffer.len() {
            return region_from_locale(&String::from_utf16_lossy(&buffer[..length as usize - 1]));
        }
    }
    #[cfg(not(windows))]
    if let Some(locale) = std::env::var_os("LC_ALL").or_else(|| std::env::var_os("LANG")) {
        return region_from_locale(&locale.to_string_lossy());
    }
    String::new()
}

fn region_from_locale(locale: &str) -> String {
    locale
        .split(['-', '_', '.'])
        .skip(1)
        .find(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
        .unwrap_or("")
        .to_ascii_uppercase()
}

const MESSAGES: &[(&str, &str)] = &[
    (
        "Для чтения этого файла нужны права администратора",
        "Administrator privileges are required to read this file",
    ),
    (
        "Слишком большое выделение. Используйте экспорт CSV",
        "The selection is too large. Use CSV export",
    ),
    (
        "Некорректный список ext: (пример: ext:exe;jar;zip)",
        "Invalid ext: list (example: ext:exe;jar;zip)",
    ),
    (
        "Не удалось запросить права администратора",
        "Could not request administrator privileges",
    ),
    ("Операция отменена", "Operation cancelled"),
    ("Операция устарела", "Operation superseded"),
    ("Нет номера запроса", "Missing request ID"),
    ("Выберите файл", "Choose a file"),
    ("Сначала откройте файл", "Open a file first"),
    ("Файл уже изменился", "The file has changed"),
    ("Файл закрыт", "The file is closed"),
    ("Представление уже изменилось", "The table view has changed"),
    ("Строка недоступна", "Row is unavailable"),
    ("Столбец недоступен", "Column is unavailable"),
    ("Неизвестная команда", "Unknown command"),
    (
        "Столбец сортировки недоступен",
        "Sort column is unavailable",
    ),
    ("Столбец фильтра недоступен", "Filter column is unavailable"),
    ("Неизвестное условие фильтра", "Unknown filter condition"),
    ("Неожиданный конец файла", "Unexpected end of file"),
    ("Сбой потока обработки", "Processing worker failed"),
    (
        "Слишком большой запрос отображения",
        "Requested viewport is too large",
    ),
    (
        "Нельзя перезаписать исходный файл",
        "The source file cannot be overwritten",
    ),
    ("Не удалось сохранить CSV", "Could not save CSV"),
    (
        "Не удалось открыть {} для чтения. Возможно, файл ещё записывается",
        "Could not open {} for reading. Another program may still be writing to it",
    ),
    (
        "Текст не в UTF-8. Выберите кодировку в параметрах открытия",
        "Text is not UTF-8. Choose the encoding in import settings",
    ),
    (
        "Заголовок не в UTF-8. Выберите кодировку",
        "Header is not UTF-8. Choose the encoding",
    ),
    (
        "Более {} столбцов. Проверьте разделитель",
        "More than {} columns. Check the delimiter",
    ),
    ("Ошибка CSV около строки {}", "CSV error near row {}"),
    (
        "Более {} столбцов. Проверьте разделитель файла",
        "More than {} columns. Check the file delimiter",
    ),
    (
        "Строка {}: данные не в UTF-8. Выберите Windows-1251, Windows-1252 или UTF-16 в параметрах открытия",
        "Row {}: data is not UTF-8. Choose Windows-1251, Windows-1252 or UTF-16 in import settings",
    ),
    ("Строка за пределами таблицы", "Row is outside the table"),
    (
        "Файл изменился или строка недоступна",
        "The file changed or the row is unavailable",
    ),
    (
        "Не удалось создать временный файл для декодирования",
        "Could not create a temporary file for decoding",
    ),
    (
        "Повреждённая последовательность в кодировке {}. Проверьте выбранную кодировку",
        "Invalid sequence in {}. Check the selected encoding",
    ),
    ("Сбой индексации CSV", "CSV indexing failed"),
    (
        "Некорректные символы после кавычки CSV",
        "Invalid characters after a CSV quote",
    ),
    (
        "Незакрытые кавычки CSV в конце файла",
        "Unclosed CSV quote at the end of the file",
    ),
    (
        "Данные не в UTF-8. Выберите кодировку в настройках",
        "Data is not UTF-8. Choose the encoding in settings",
    ),
    (
        "Некорректные символы после закрывающей кавычки CSV, байт {}",
        "Invalid characters after a closing CSV quote at byte {}",
    ),
    (
        "JSON содержит более {} столбцов",
        "JSON contains more than {} columns",
    ),
    ("Ошибка массива JSON", "Invalid JSON array"),
    (
        "Лишние данные после массива JSON",
        "Unexpected data after the JSON array",
    ),
    ("Ошибка JSON / JSON Lines", "Invalid JSON / JSON Lines"),
    ("Введите текст для поиска", "Enter search text"),
    (
        "Некорректное регулярное выражение",
        "Invalid regular expression",
    ),
    (
        "Ошибка regex в фильтре",
        "Invalid regular expression in filter",
    ),
    (
        "Слишком большой индекс результатов",
        "Result index is too large",
    ),
    ("Ячейка недоступна", "Cell is unavailable"),
    (
        "Не удалось создать временный файл поиска",
        "Could not create a temporary search file",
    ),
    (
        "Неожиданный конец файла при поиске",
        "Unexpected end of file while searching",
    ),
    ("Сбой потока поиска", "Search worker failed"),
];

fn translate(message: &str) -> String {
    for &(russian, english) in MESSAGES {
        if let Some((start, end)) = russian.split_once("{}") {
            if let Some(value) = message
                .strip_prefix(start)
                .and_then(|s| s.strip_suffix(end))
            {
                return english.replacen("{}", value, 1);
            }
        } else if message == russian {
            return english.to_owned();
        }
    }
    message.to_owned()
}

pub fn headers(data: &crate::data::Dataset, language: Language) -> Vec<String> {
    use std::collections::{HashMap, HashSet};
    let generated: HashMap<_, _> = data
        .generated_headers
        .iter()
        .map(|item| (item.column, item.kind))
        .collect();
    let mut used: HashSet<String> = data
        .headers
        .iter()
        .enumerate()
        .filter(|(i, _)| !generated.contains_key(i))
        .map(|(_, name)| name.clone())
        .collect();
    data.headers
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let Some(kind) = generated.get(&i) else {
                return name.clone();
            };
            let base = if *kind == "text" {
                language.text("Текст", "Text").to_owned()
            } else {
                format!("{} {}", language.text("Столбец", "Column"), i + 1)
            };
            let mut label = base.clone();
            let mut suffix = 2;
            while used.contains(&label) {
                label = format!("{base} ({suffix})");
                suffix += 1;
            }
            used.insert(label.clone());
            label
        })
        .collect()
}

pub fn english_error(error: &anyhow::Error) -> String {
    error
        .chain()
        .map(|cause| {
            if let Some(io) = cause.downcast_ref::<std::io::Error>() {
                // OS error text follows Windows' language, not the app language.
                let summary = match io.kind() {
                    std::io::ErrorKind::NotFound => "File or directory not found",
                    std::io::ErrorKind::PermissionDenied => "Access denied",
                    std::io::ErrorKind::AlreadyExists => "File already exists",
                    std::io::ErrorKind::InvalidData => "Invalid data",
                    std::io::ErrorKind::UnexpectedEof => "Unexpected end of file",
                    std::io::ErrorKind::WriteZero => "Could not write data",
                    std::io::ErrorKind::StorageFull => "Not enough disk space",
                    _ => "I/O error",
                };
                return match io.raw_os_error() {
                    Some(code) => format!("{summary} (OS error {code})"),
                    None => summary.to_owned(),
                };
            }
            translate(&cause.to_string())
        })
        .collect::<Vec<_>>()
        .join(": ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn country_controls_default_not_display_language() {
        for country in ["RU", "BY", "UA", "ru"] {
            assert_eq!(Language::from_region(country), Language::Russian);
        }
        for country in ["US", "GB", "KZ", "DE", "", "001"] {
            assert_eq!(Language::from_region(country), Language::English);
        }
        assert_eq!(region_from_locale("en-RU"), "RU");
        assert_eq!(region_from_locale("ru-US"), "US");
        assert_eq!(region_from_locale("uk_UA.UTF-8"), "UA");
        assert_eq!(region_from_locale("ru"), "");
    }
    #[test]
    fn translated_errors_preserve_paths_and_values() {
        let path = r"C:\Текст\Ошибка CSV.csv";
        let error = anyhow::anyhow!(
            "Не удалось открыть {path} для чтения. Возможно, файл ещё записывается"
        )
        .context("Ошибка CSV около строки 42");
        assert_eq!(
            english_error(&error),
            format!(
                "CSV error near row 42: Could not open {path} for reading. Another program may still be writing to it"
            )
        );
        assert_eq!(
            english_error(&anyhow::anyhow!("Операция отменена")),
            "Operation cancelled"
        );
    }
}
