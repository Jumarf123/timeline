// App-owned strings only. Imported headers and cell values are never translated.
(() => {
  "use strict";
  const english = {
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
  setLanguage(preference);
  window.timelineI18n = {
    t,
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
