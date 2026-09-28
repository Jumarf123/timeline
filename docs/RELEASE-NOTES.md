# Timeline 0.2.0 — Windows x64

## Русский

Портативный локальный просмотрщик таблиц, журналов и forensic-файлов для Windows 10/11 x64. Запустите `Timeline.exe`; нужен установленный Microsoft Edge WebView2 Runtime. Данные не отправляются в интернет.

- Окно сразу открывается развёрнутым, с обычной рамкой и панелью задач Windows.

- Поиск: режимы Авто, Текст и Regex, флаги `(?i)` / `(?-i)`, запросы `ext:exe;jar regex:…`. Поиск проверяет полные значения во всём файле.
- JSON и JSONL: дерево объектов и массивов, форматированный и исходный текст, сохранение точности больших чисел.
- Markdown: чтение документа и переключение на исходник. JavaScript, Python, PowerShell, XML, YAML и другие текстовые файлы: подсветка, отступы, номера строк, сворачивание блоков и поиск в исходнике.
- Выделение прямоугольного диапазона мышью, Shift+щелчком и Shift+стрелками. Ctrl+C копирует полные значения с учётом порядка столбцов и текущей сортировки.
- Автоматическое распознавание текстовых таблиц; его можно отключить в настройках.
- Нетабличный TXT: режим документа по умолчанию, перенос строк с сохранением настройки, поиск и копирование выделенного текста.
- Читаемые бинарные поля EDB/DAT/SQLite/ARN: UTF-8 и UTF-16 строки, включая URL после бинарного заголовка. Оригинальный hex хранится отдельно; для неизвестных DAT доступны извлечённые строки с точными смещениями.
- Просмотр текстовых hex-значений в любых таблицах: списки байтов, `0x`, `\x`, SQL и другие формы с переключением между читаемым текстом и оригиналом.
- EVTX, XML, Autoruns ARN, ESE/EDB, registry DAT, SQLite и USN: автоматический разбор. Неизвестные бинарные файлы доступны как байты и строки.
- Отдельные этапы открытия: чтение, декодирование, разбор, индексирование и подготовка таблицы. Счётчики записей, времени и возможность отменить работу.
- EVTX обрабатывается параллельно по блокам с сохранением порядка. Ошибки повреждённых записей и предупреждения импорта показываются явно.
- Занятые файлы: временная копия через доступные Windows API; при необходимости — запрос UAC и повторное открытие с правами администратора. Исходный файл не редактируется.

Особенности форматов, примеры поиска и ограничения описаны в `formats-and-search.md` (в исходниках — `docs/FORMATS.md`).

## English

Portable local table, log and forensic artifact viewer for Windows 10/11 x64. Run `Timeline.exe`; Microsoft Edge WebView2 Runtime is required. File contents stay on your computer.

- Starts maximized with the normal Windows title bar and taskbar.

- Auto, literal and regex search, inline case flags and `ext:exe;jar regex:…` queries across complete cell values and all rows.
- JSON/JSONL trees, formatted and original source with lossless numbers; Markdown preview and source; highlighted source code with folding and line numbers.
- Rectangular selection and TSV clipboard copying in the current row and column order.
- Optional automatic text-table detection and importers for EVTX, XML, Autoruns ARN, ESE/EDB, registry hives, SQLite and USN. Hexadecimal fallback for unknown binary data.
- Visible import phases, record counts, elapsed time and cancellation; parallel EVTX decoding that preserves record order and reports corrupt records.
- Windows snapshot/acquisition paths for busy files, with an explicit UAC prompt when elevation is needed.

See the format and search guide for supported variants and practical limits.
