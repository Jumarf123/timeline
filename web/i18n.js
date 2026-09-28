// App-owned strings only. Imported headers and cell values are never translated.
(() => {
  "use strict";
  const english = {
    "Строк: {count}": "Lines: {count}",
    "Вид значения": "Value view",
    "Читаемый текст": "Readable text",
    "Исходное значение": "Original value",
    "Исходные байты": "Original bytes",
    "Читаемый вид": "Readable view",
    "Бинарный файл: смещения, шестнадцатеричные байты и ASCII. Структура формата не интерпретируется.":
      "Binary file: byte offsets, hexadecimal data and ASCII. No format-specific interpretation.",
    "Извлечённые строки: вероятный текст UTF-8/UTF-16 с исходными смещениями. Структура формата интерпретирована не полностью. Кнопка «Исходные байты» показывает все байты, включая нераспознанные данные.":
      "Binary strings view: probable UTF-8/UTF-16 text with original byte offsets, not a complete interpretation of this file format. Open as Hex to inspect every original byte, including unrecognized data.",
    "Читаемый текст не обнаружен. Кнопка «Исходные байты» показывает все исходные данные.":
      "No confidently readable text was found. Open as Hex to inspect all original bytes.",
    "Очень длинные строки разделены на фрагменты, чтобы ограничить потребление памяти.":
      "Very long text spans are split at read-window boundaries to keep memory use bounded.",
    "Двоичные значения показывают вероятный читаемый текст; Raw data сохраняет исходные шестнадцатеричные байты.":
      "Binary values show probable readable strings when present; Raw data retains the original hexadecimal bytes.",
    "Двоичные значения показывают вероятный читаемый текст; столбцы $raw сохраняют исходные шестнадцатеричные байты.":
      "Binary values show probable readable strings when present; $raw columns retain their original hexadecimal bytes.",
    "Снимок реестра: журналы транзакций и удалённые ключи не восстанавливаются.":
      "Registry hive snapshot; transaction logs and deleted keys are not replayed.",
    "Снимок ESE: журналы транзакций не применяются. Вынесенные или сжатые длинные значения могут оставаться двоичными. Двоичные значения показывают вероятный читаемый текст; столбцы $raw сохраняют исходные шестнадцатеричные байты.":
      "ESE snapshot: transaction logs are not replayed. Separated/compressed long values may remain binary. Binary values show probable readable strings when present; $raw columns retain their original hexadecimal bytes.",
    "Снимок основной базы SQLite: отдельные файлы WAL не применяются. Двоичные значения показывают вероятный читаемый текст; столбцы $raw сохраняют исходные шестнадцатеричные байты.":
      "SQLite main database snapshot; separate WAL files are not applied. Binary values show probable readable strings when present; $raw columns retain their original hexadecimal bytes.",
    "Неизвестные двоичные поля Autoruns показывают вероятный читаемый текст; столбцы $raw сохраняют исходные шестнадцатеричные байты.":
      "Unknown Autoruns binary fields show probable readable strings when present; $raw columns retain their original hexadecimal bytes.",
    "Не удалось полностью разобрать {format}: {error}. Показаны извлечённые строки. Все исходные данные доступны по кнопке «Исходные байты».":
      "Could not fully parse {format}: {error}. Showing extracted strings; original bytes are available in Hex mode.",
    "Перенос строк": "Word wrap",
    "Текст документа": "Document text",
    Найти: "Find",
    "Все совпадения": "All matches",
    "Слово целиком": "Whole word",
    "Перейти к строке": "Go to line",
    Перейти: "Go",
    "Поиск в текущем фрагменте (Ctrl+F)": "Search this part (Ctrl+F)",
    "Копировать выделение или текущий фрагмент":
      "Copy the selection or the current part",
    "Копировать текст": "Copy text",
    "Ожидаем подтверждения UAC…": "Waiting for UAC confirmation…",
    "Дерево недоступно для этого фрагмента JSON. Показан исходный текст без потери данных.":
      "The JSON tree is unavailable for this part. The original text is shown without data loss.",
    "Читаем файл…": "Reading file…",
    "Преобразуем кодировку…": "Converting encoding…",
    "Разбираем записи…": "Parsing records…",
    "Подготавливаем таблицу…": "Preparing table…",
    "Определяем типы столбцов…": "Detecting column types…",
    "Записей: {count}": "Records: {count}",
    "{seconds} с": "{seconds} s",
    "Байты (Hex)": "Bytes (Hex)",
    "Не удалось разобрать файл": "Could not parse the file",
    "Открыть как байты": "Open as bytes",
    "Режим поиска": "Search mode",
    Авто: "Auto",
    "Авто распознаёт regex:, ext: и (?i). Остальной текст ищется буквально.":
      "Auto recognizes regex:, ext: and (?i). Other text is matched literally.",
    "Распознавать таблицы в TXT автоматически":
      "Automatically detect tables in TXT",
    Просмотр: "Viewer",
    Таблица: "Table",
    "Копировать выделенные ячейки": "Copy selected cells",
    "Вид документа": "Document view",
    "Исходный текст": "Source text",
    "Дерево JSON": "JSON tree",
    Предпросмотр: "Preview",
    "Без подсветки": "No highlighting",
    "Показать исходный вид": "Show raw source",
    Назад: "Previous",
    Далее: "Next",
    "Показать ещё": "Show more",
    "Нужны права администратора": "Administrator access required",
    "Windows ограничила доступ к этому файлу. Откройте отдельное окно Timeline с правами администратора и подтвердите запрос UAC.":
      "Windows restricted access to this file. Open a separate Timeline window as administrator and confirm the UAC prompt.",
    "Выбрать другой файл": "Choose another file",
    "Открыть от имени администратора": "Open as administrator",
    "Фрагмент {number} · поиск по всему файлу — в таблице":
      "Part {number} · use the table to search the entire file",
    "1 000": "1,000",
    "5 000": "5,000",
    "Открыть файл": "Open file",
    Настройки: "Settings",
    "Выбрать файл": "Choose file",
    "или перетащите его в это окно": "or drop it into this window",
    "Параметры открытия": "Import settings",
    Открыть: "Open",
    "Найти во всей таблице…": "Search the entire table…",
    "Поиск по всем строкам": "Search all rows",
    "Очистить поиск": "Clear search",
    Фильтры: "Filters",
    Столбцы: "Columns",
    Экспорт: "Export",
    "Таблица. Стрелки — выбрать ячейку, Enter — открыть, Ctrl+C — скопировать.":
      "Table. Arrow keys select a cell, Enter opens it, Ctrl+C copies it.",
    "Ничего не найдено": "No results",
    "Попробуйте другой запрос или уберите условия фильтра.":
      "Try another search or remove a filter.",
    "Сбросить поиск и фильтры": "Clear search and filters",
    "Обрабатываем все строки…": "Processing all rows…",
    "Строк на странице": "Rows per page",
    "Все строки": "All rows",
    "Предыдущая страница": "Previous page",
    "Следующая страница": "Next page",
    "Содержимое ячейки": "Cell contents",
    "Закрыть просмотр ячейки": "Close cell viewer",
    "Показан первый миллион символов. Полное значение доступно в экспорте.":
      "Showing the first million characters. Export includes the complete value.",
    Копировать: "Copy",
    Отменить: "Cancel",
    "Закрыть сообщение": "Dismiss message",
    "Отпустите, чтобы открыть файл": "Drop to open the file",
    Закрыть: "Close",
    Текст: "Text",
    Число: "Number",
    "Дата и время": "Date and time",
    "Unix / дата": "Unix / date",
    Содержит: "Contains",
    "Не содержит": "Does not contain",
    Равно: "Equals",
    "Не равно": "Does not equal",
    "Начинается с": "Starts with",
    Пусто: "Is empty",
    "Не пусто": "Is not empty",
    "Регулярное выражение": "Regular expression",
    "Откройте Timeline.exe для работы с файлами.":
      "Open Timeline.exe to work with files.",
    "Не удалось выполнить операцию": "The operation failed",
    "Открываем файл…": "Opening file…",
    "Ищем и сортируем по всему файлу…":
      "Searching and sorting the entire file…",
    "{count} строк имеют разное число полей. Отсутствующие значения показаны пустыми.":
      "{count} rows have a different number of fields. Missing values are shown as empty.",
    "{name} · {type}. Нажмите для сортировки, потяните для перемещения":
      "{name} · {type}. Click to sort, drag to move",
    "Сортировать: {name}": "Sort: {name}",
    Название: "Name",
    "Столбец {number}": "Column {number}",
    "Открепить первый столбец": "Unpin first column",
    "Закрепить первый столбец": "Pin first column",
    "Настроить столбец {name}": "Configure column {name}",
    "Потяните, чтобы изменить ширину. Двойной щелчок — автоподбор":
      "Drag to resize. Double-click to fit contents",
    "{start}–{end} из {total}": "{start}–{end} of {total}",
    "0 строк": "0 rows",
    "{count} строк": "{count} rows",
    "Найдено {count} из {total} строк": "Found {count} of {total} rows",
    "В файле пока нет строк": "This file has no rows",
    "Убрать: {text}": "Remove: {text}",
    "Поиск: {text}": "Search: {text}",
    "по убыванию": "descending",
    "по возрастанию": "ascending",
    "Сбросить всё": "Clear all",
    "Дождитесь загрузки строки": "Wait for the row to load",
    "Загрузка…": "Loading…",
    "{name} · строка {number}": "{name} · row {number}",
    "Не удалось скопировать. Выделите текст в панели и нажмите Ctrl+C.":
      "Could not copy. Select the text in the panel and press Ctrl+C.",
    Скопировано: "Copied",
    "Настройте отображение и параметры открытия файлов.":
      "Choose display and import settings.",
    "Язык интерфейса": "Interface language",
    "Применяется сразу после сохранения": "Applies when you save",
    "Автоматически (регион системы)": "Automatic (system region)",
    "«Все строки» — одна непрерывная таблица":
      "“All rows” shows one continuous table",
    "Высота строк": "Row height",
    "Выберите удобную плотность": "Choose a comfortable density",
    Обычная: "Normal",
    Компактная: "Compact",
    "Все колонки в полную длину": "Fit all columns to contents",
    "Учитывать регистр": "Case sensitive",
    "Регулярные выражения": "Regular expressions",
    "При следующем открытии файла": "For the next file you open",
    Кодировка: "Encoding",
    "Если вместо текста отображаются неверные символы":
      "Use this if text displays incorrectly",
    Автоматически: "Automatic",
    "Разделитель CSV": "CSV delimiter",
    "Обычно определяется автоматически": "Usually detected automatically",
    Запятая: "Comma",
    "Точка с запятой": "Semicolon",
    Табуляция: "Tab",
    "Вертикальная черта": "Pipe",
    "Формат файла": "File format",
    "CSV, JSON или построчный текст": "CSV, JSON or line-based text",
    "Текст / журнал": "Text / log",
    "Первая строка файла содержит заголовки": "First row contains headers",
    Отмена: "Cancel",
    Сохранить: "Save",
    "Выберите, что показывать. Столбец с названием записи всегда включён.":
      "Choose which columns to show. The record name column is always included.",
    "Найти столбец…": "Find a column…",
    "Найти столбец": "Find a column",
    "Показать все": "Show all",
    Применить: "Apply",
    "Добавьте условие, чтобы оставить только нужные строки.\nВсе условия применяются вместе.":
      "Add a condition to keep the rows you need.\nAll conditions apply together.",
    "Столбец фильтра": "Filter column",
    "Условие фильтра": "Filter condition",
    Значение: "Value",
    "Значение фильтра": "Filter value",
    "Удалить условие": "Remove condition",
    "Условия применяются ко всему файлу, включая строки на других страницах.":
      "Conditions apply to the entire file, including rows on other pages.",
    "+ Добавить условие": "+ Add condition",
    Сбросить: "Reset",
    "Сортировка и фильтр по всему файлу.": "Sort and filter the entire file.",
    "По возрастанию": "Ascending",
    "По убыванию": "Descending",
    "Сравнивать как": "Compare as",
    "Тип определяется по значениям столбца": "Detected from the column values",
    "Авто · {type}": "Auto · {type}",
    "Даты вида 03/04/2026 читаются как 3 апреля. Пустые значения остаются в конце списка.":
      "Dates such as 03/04/2026 mean 3 April. Empty values stay at the end.",
    "Сохраняем CSV…": "Saving CSV…",
    "Сохранено {count} строк: {path}": "Saved {count} rows: {path}",
  };
  const resolveLanguage = (preference, region) =>
    ["ru", "en"].includes(preference)
      ? preference
      : ["RU", "BY", "UA"].includes(String(region).toUpperCase())
        ? "ru"
        : "en";
  let preference = "auto";
  try {
    preference =
      JSON.parse(localStorage.getItem("timeline.settings") || "{}").language ||
      "auto";
  } catch {}
  let language, numbers;
  const staticText = [],
    staticAttributes = [];
  function setLanguage(value) {
    language = resolveLanguage(value, window.timelineRegion);
    numbers = new Intl.NumberFormat(language === "ru" ? "ru-RU" : "en-US");
    document.documentElement.lang = language;
  }
  function t(key, values = {}) {
    const template = language === "en" ? (english[key] ?? key) : key;
    return template.replace(
      /\{(\w+)\}/g,
      (match, name) => values[name] ?? match,
    );
  }
  function collectStatic() {
    const walker = document.createTreeWalker(
      document.body,
      NodeFilter.SHOW_TEXT,
    );
    while (walker.nextNode()) {
      const node = walker.currentNode;
      if (node.parentElement.closest("script, style")) continue;
      const key = node.textContent.trim().replace(/\s+/g, " ");
      if (Object.hasOwn(english, key)) staticText.push({ node, key });
    }
    for (const node of document.querySelectorAll(
      "[title], [placeholder], [aria-label]",
    ))
      for (const attribute of ["title", "placeholder", "aria-label"]) {
        const key = node.getAttribute(attribute);
        if (Object.hasOwn(english, key))
          staticAttributes.push({ node, attribute, key });
      }
    applyStatic();
  }
  function applyStatic() {
    for (const { node, key } of staticText)
      if (node.isConnected) node.textContent = t(key);
    for (const { node, attribute, key } of staticAttributes)
      if (node.isConnected) node.setAttribute(attribute, t(key));
  }
  const russianDiagnostics = new Map(
    Object.entries(english).map(([ru, en]) => [en, ru]),
  );
  function diagnostic(message) {
    if (language !== "ru") return message;
    const fallback = message.match(
      /^Could not fully parse (.+?): ([\s\S]+)\. Showing extracted strings; original bytes are available in Hex mode\.$/,
    );
    return (
      russianDiagnostics.get(message) ||
      (fallback
        ? t(
            "Не удалось полностью разобрать {format}: {error}. Показаны извлечённые строки. Все исходные данные доступны по кнопке «Исходные байты».",
            { format: fallback[1], error: fallback[2] },
          )
        : message)
    );
  }
  setLanguage(preference);
  window.timelineI18n = {
    t,
    diagnostic,
    setLanguage,
    collectStatic,
    applyStatic,
    resolveLanguage,
    get language() {
      return language;
    },
    number: (value) => numbers.format(value),
    english,
  };
})();
