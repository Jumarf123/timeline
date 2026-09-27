# Building Timeline / Сборка Timeline

<a id="ru"></a>

## Русский

Нужны Windows x64, Rust 1.88+ с MSVC toolchain, Visual Studio Build Tools с компонентами C++ и Windows SDK. Для запуска нужен WebView2 Runtime. Node.js 22+ нужен только для проверок интерфейса.

```powershell
git clone https://github.com/Jumarf123/timeline.git
cd timeline
cargo build --release --locked
```

Готовый файл: `target/release/timeline.exe`. HTML, CSS и JavaScript встроены в exe; сервер и Node.js для запуска не нужны.

Проверки:

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
npm ci
npm run test:i18n
$env:TIMELINE_EXE = (Resolve-Path target/release/timeline.exe).Path
npm test
```

Сквозной тест запускает настоящий exe с отдельным профилем WebView2 и таблицей на 500 257 строк. Проверяются поиск, сортировка, фильтры, страницы, перетаскивание, масштаб и переключение языков. Скриншоты и временные файлы остаются в `test-results/`, который исключён из Git.

Основные части: `src/data.rs` — импорт и индекс, `src/query.rs` — запросы и экспорт, `src/bridge.rs` — обмен с интерфейсом, `src/locale.rs` — регион Windows и нативные сообщения, `web/` — интерфейс и переводы. Исходные строки читаются по мере необходимости; сортировка временно хранит ключи и позиции строк.

Для воспроизводимых языковых проверок задайте `TIMELINE_REGION=RU`, `BY`, `UA` или `US` только для тестового процесса. `TIMELINE_PROFILE` выбирает отдельный профиль. Обычный запуск читает регион Windows и использует `%LOCALAPPDATA%/Timeline/WebView`.

<a id="en"></a>

## English

Requirements: x64 Windows, Rust 1.88+ with the MSVC toolchain, Visual Studio Build Tools with C++ and Windows SDK components. WebView2 Runtime is required to run the app. Node.js 22+ is only needed for UI tests.

```powershell
git clone https://github.com/Jumarf123/timeline.git
cd timeline
cargo build --release --locked
```

The executable is `target/release/timeline.exe`. HTML, CSS and JavaScript are embedded; running it does not require a server or Node.js.

Use the verification commands above. The end-to-end suite launches the real executable with an isolated WebView2 profile and a 500,257-row fixture. It covers search, sorting, filters, pagination, dragging, zoom and language changes. Screenshots and temporary files go to the ignored `test-results/` directory.

Key modules: `src/data.rs` handles import and indexing; `src/query.rs` queries and export; `src/bridge.rs` frontend communication; `src/locale.rs` Windows region and native messages; `web/` the UI and translations. Rows are read on demand; sorting temporarily stores keys and row positions.

For deterministic tests, set `TIMELINE_REGION` to RU, BY, UA or US for the test process. `TIMELINE_PROFILE` selects a separate profile. Normal launches read the Windows region and use `%LOCALAPPDATA%/Timeline/WebView`.
