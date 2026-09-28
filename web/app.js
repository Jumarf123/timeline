(() => {
  "use strict";
  const i18n = window.timelineI18n;
  const { t } = i18n;
  i18n.collectStatic();
  const $ = (id) => document.getElementById(id);
  const icons = {
    folder:
      '<path d="M3 7V5a1 1 0 0 1 1-1h5l2 3h9a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1Z"/><path d="M3 8h18"/>',
    settings:
      '<path d="m9 3-.6 2.2-2 .9L4.3 5.5 2 9.5l1.6 1.6v2L2 14.5l2.3 4 2.1-.6 2 .9L9 21h6l.6-2.2 2-.9 2.1.6 2.3-4-1.6-1.4v-2L22 9.5l-2.3-4-2.1.6-2-.9L15 3Z"/><circle cx="12" cy="12" r="3"/>',
    search: '<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/>',
    table:
      '<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 9h18M3 15h18M9 9v12"/>',
    columns:
      '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16M15 4v16"/>',
    filter: '<path d="M3 5h18M6 12h12M10 19h4"/>',
    download: '<path d="M12 3v12m-4-4 4 4 4-4M4 15v5h16v-5"/>',
    close: '<path d="m6 6 12 12M6 18 18 6"/>',
    left: '<path d="m14 6-6 6 6 6"/>',
    right: '<path d="m10 6 6 6-6 6"/>',
    pin: '<path d="m9 3 6 0-1 6 4 4H6l4-4ZM12 13v8"/>',
    copy: '<rect x="8" y="8" width="12" height="13" rx="2"/><path d="M15 8V3H3v12h5"/>',
    sort: '<path d="M8 4v16m-3-3 3 3 3-3M16 20V4m-3 3 3-3 3 3"/>',
    up: '<path d="M12 20V4m-5 5 5-5 5 5"/>',
    down: '<path d="M12 4v16m-5-5 5 5 5-5"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
  };
  const svg = (name) =>
    `<svg viewBox="0 0 24 24" aria-hidden="true">${icons[name] || icons.table}</svg>`;
  document
    .querySelectorAll("[data-icon]")
    .forEach((el) => el.insertAdjacentHTML("afterbegin", svg(el.dataset.icon)));
  const el = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const fmt = (n) => i18n.number(n);
  const typeLabels = {
    text: "Текст",
    number: "Число",
    date: "Дата и время",
    timestamp: "Unix / дата",
  };
  const ops = {
    contains: "Содержит",
    notContains: "Не содержит",
    equals: "Равно",
    notEquals: "Не равно",
    startsWith: "Начинается с",
    empty: "Пусто",
    notEmpty: "Не пусто",
    regex: "Регулярное выражение",
  };
  const defaults = {
    language: "auto",
    pageSize: 0,
    compact: false,
    fullWidthColumns: true,
    pinFirstColumn: false,
    interfaceZoom: 1,
    regex: false,
    searchMode: "auto",
    autoText: true,
    caseSensitive: false,
    header: true,
    encoding: "auto",
    format: "auto",
    delimiter: "",
    documentWrap: true,
  };
  let settings;
  try {
    settings = {
      ...defaults,
      ...JSON.parse(localStorage.getItem("timeline.settings") || "{}"),
    };
  } catch {
    settings = { ...defaults };
  }
  if (![0, 100, 500, 1000, 5000].includes(Number(settings.pageSize)))
    settings.pageSize = 0;
  if (!["auto", "ru", "en"].includes(settings.language))
    settings.language = "auto";
  if (settings.regex) {
    settings.searchMode = "regex";
    settings.regex = false;
  }
  if (!["auto", "text", "regex"].includes(settings.searchMode))
    settings.searchMode = "auto";
  const zoomLevels = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];
  if (!zoomLevels.includes(settings.interfaceZoom)) settings.interfaceZoom = 1;
  const state = {
    file: null,
    revision: 0,
    total: 0,
    rows: 0,
    headers: [],
    types: [],
    visible: [],
    columnOrder: [],
    widths: [],
    contentWidths: [],
    manualWidths: [],
    hoverColumn: null,
    page: 0,
    filters: [],
    sort: null,
    text: "",
    applied: { filters: [], sort: null, text: "" },
    selected: null,
    anchor: null,
    cache: new Map(),
    cacheKey: "",
    loading: false,
    loads: new Set(),
    readError: false,
    busy: 0,
    jobKind: "",
    queryVersion: 0,
    frame: 0,
    picking: false,
    inspectorVersion: 0,
    cell: null,
    document: null,
    documentOffsets: [0],
    documentLines: [1],
    documentPage: 0,
    documentVersion: 0,
  };
  let sequence = 0,
    searchTimer,
    noticeTimer;
  const pending = new Map();
  const viewport = $("viewport");
  const grid = $("data-grid");
  const canvas = $("canvas");
  const measure = document.createElement("canvas").getContext("2d");
  const tableFontFamily = getComputedStyle(document.body).fontFamily;
  const measuredText = new Map();
  function textWidth(value, weight = 400) {
    const text = value.replace(/\s+/g, " ");
    const key = weight + ":" + text;
    if (measuredText.has(key)) return measuredText.get(key);
    measure.font = `${weight} 12px ${tableFontFamily}`;
    const width = Math.ceil(measure.measureText(text).width);
    if (measuredText.size >= 4096) measuredText.clear();
    measuredText.set(key, width);
    return width;
  }
  function fitRow(row, columns) {
    if (!settings.fullWidthColumns) return;
    for (const column of columns) {
      const text = row.cells[column];
      if (text === undefined) continue;
      // Include cell padding, the original row number, and a little font rounding room.
      const padding =
        column === 0
          ? 83 + Math.max(0, String(state.total).length - 6) * 7
          : 32;
      state.contentWidths[column] = Math.max(
        state.contentWidths[column] || 0,
        textWidth(displayCell(text, column, row), 500) + padding,
      );
    }
  }
  function displayCell(text, column, row) {
    // Only the binary parser's explicit placeholder is a UI message.
    return state.file?.kind === "binary" &&
      column === 3 &&
      state.headers[2] === "Encoding" &&
      row?.cells[2] === "Binary"
      ? i18n.diagnostic(text)
      : text;
  }
  function firstColumn() {
    return (
      state.columnOrder.find(
        (column) => state.visible[column] || column === 0,
      ) ?? 0
    );
  }
  function columnWidth(column) {
    const first = firstColumn();
    const numberPadding =
      column === first && column !== 0
        ? 52 + Math.max(0, String(state.total).length - 6) * 7
        : 0;
    const automatic = settings.fullWidthColumns
      ? Math.max(
          state.widths[column],
          (state.contentWidths[column] || 0) + numberPadding,
        )
      : state.widths[column];
    const manual = state.manualWidths[column];
    if (column !== first || !settings.pinFirstColumn)
      return manual ?? automatic;
    // Keep the entire name column pinned while leaving room for the other columns.
    const available = Math.max(120, viewport.clientWidth - 120);
    return Math.min(
      manual ?? Math.min(automatic, Math.max(160, viewport.clientWidth * 0.45)),
      available,
    );
  }
  function frozenWidth() {
    return settings.pinFirstColumn ? columnWidth(firstColumn()) : 0;
  }
  function saveSettings() {
    try {
      localStorage.setItem("timeline.settings", JSON.stringify(settings));
    } catch {}
  }
  function setZoom(factor) {
    if (factor === settings.interfaceZoom) return;
    cancelColumnDrag();
    settings.interfaceZoom = factor;
    saveSettings();
    window.ipc?.postMessage(JSON.stringify({ command: "zoom", factor }));
  }
  function stepZoom(direction) {
    const index = zoomLevels.indexOf(settings.interfaceZoom);
    setZoom(
      zoomLevels[
        Math.max(0, Math.min(zoomLevels.length - 1, index + direction))
      ],
    );
  }
  function hoverColumn(column) {
    if (state.hoverColumn === column) return;
    state.hoverColumn = column;
    grid.querySelectorAll("[data-column]").forEach((cell) => {
      cell.classList.toggle(
        "column-hover",
        Number(cell.dataset.column) === column,
      );
    });
  }
  function rpc(command, params = {}) {
    const id = ++sequence;
    const promise = new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      if (!window.ipc) {
        pending.delete(id);
        reject(new Error(t("Откройте Timeline.exe для работы с файлами.")));
        return;
      }
      window.ipc.postMessage(
        JSON.stringify({ id, command, language: i18n.language, ...params }),
      );
    });
    promise.id = id;
    return promise;
  }
  window.timelineReceive = (message) => {
    if (message.event === "open") {
      $("drop-overlay").hidden = true;
      openFile(message.path);
      return;
    }
    if (message.event === "drag") {
      $("drop-overlay").hidden = !message.active;
      return;
    }
    if (message.event === "progress") {
      if (message.id !== state.busy) return;
      state.jobProgress = message;
      renderProgress(message);
      return;
    }
    const handler = pending.get(message.id);
    if (!handler) return;
    pending.delete(message.id);
    if (message.ok) handler.resolve(message.data);
    else
      handler.reject(
        Object.assign(
          new Error(
            (i18n.language === "en" ? message.error_en : message.error) ||
              t("Не удалось выполнить операцию"),
          ),
          {
            cancelled: message.cancelled,
            needsElevation: message.needs_elevation,
            canOpenRaw: message.can_open_raw,
          },
        ),
      );
  };
  function notify(text, error = false) {
    clearTimeout(noticeTimer);
    $("notice-text").textContent = text;
    $("notice").classList.toggle("error", error);
    $("notice").hidden = false;
    if (!error)
      noticeTimer = setTimeout(() => ($("notice").hidden = true), 4500);
  }
  function jobLabel(kind) {
    return t(
      kind === "open"
        ? "Открываем файл…"
        : kind === "export"
          ? "Сохраняем CSV…"
          : "Обрабатываем все строки…",
    );
  }
  function renderProgress(message) {
    const phases = {
      read: "Читаем файл…",
      decode: "Преобразуем кодировку…",
      parse: "Разбираем записи…",
      index: "Подготавливаем таблицу…",
      prepare: "Определяем типы столбцов…",
    };
    const kind = message.kind || state.jobKind;
    $("job-label").textContent =
      kind === "open" && phases[message.phase]
        ? t(phases[message.phase])
        : jobLabel(kind);
    $("job-bar").dataset.phase = message.phase || kind;
    const fraction = Number.isFinite(message.fraction)
      ? Math.max(0, Math.min(1, message.fraction))
      : null;
    if (typeof fraction === "number") $("job-progress").value = fraction;
    else $("job-progress").removeAttribute("value");
    const parts = [];
    if (typeof message.records === "number")
      parts.push(t("Записей: {count}", { count: fmt(message.records) }));
    if (typeof fraction === "number")
      parts.push(Math.min(99, Math.floor(fraction * 100)) + "%");
    parts.push(
      t("{seconds} с", { seconds: fmt(Math.floor(message.elapsed || 0)) }),
    );
    $("job-percent").textContent = parts.join(" · ");
  }
  function displayHeaders(file) {
    const generated = new Map(
      (file.generated_headers || []).map((item) => [item.column, item.kind]),
    );
    const used = new Set(file.headers.filter((_, i) => !generated.has(i)));
    return file.headers.map((name, i) => {
      if (!generated.has(i)) return name;
      const base =
        generated.get(i) === "text"
          ? t("Текст")
          : t("Столбец {number}", { number: i + 1 });
      let label = base,
        suffix = 2;
      while (used.has(label)) label = base + " (" + suffix++ + ")";
      used.add(label);
      return label;
    });
  }
  function applyLanguage() {
    cancelColumnDrag();
    i18n.setLanguage(settings.language);
    i18n.applyStatic();
    if (state.file) {
      state.headers = displayHeaders(state.file);
      state.headers.forEach((name, column) => {
        state.contentWidths[column] = Math.max(
          state.contentWidths[column] || 0,
          textWidth(name, 600) + (column === 0 ? 66 : 104),
        );
      });
    }
    $("table-header").dataset.key = "";
    if (state.file) {
      $("binary-toggle").textContent = t(
        state.file.kind === "hex" ? "Читаемый вид" : "Исходные байты",
      );
      $("document-toggle").textContent = t(
        $("document-pane").hidden ? "Просмотр" : "Таблица",
      );
    }
    if (state.busy)
      renderProgress(state.jobProgress || { kind: state.jobKind });
    $("notice").hidden = true;
    updateControls();
    if (!$("inspector").hidden && state.selected)
      inspect(state.selected.index, state.selected.column);
  }
  function busy(promise, kind, label) {
    state.busy = promise.id;
    state.jobKind = kind;
    state.jobProgress = null;
    $("job-label").textContent = label;
    $("job-progress").removeAttribute("value");
    $("job-percent").textContent = "";
    $("job-bar").hidden = false;
    $("table-busy").hidden = !state.file;
    $("export").disabled = true;
    return promise;
  }
  function finish(id) {
    if (state.busy !== id) return;
    state.busy = 0;
    state.jobKind = "";
    $("job-bar").hidden = true;
    $("table-busy").hidden = true;
    $("export").disabled = false;
    schedule();
  }
  async function pickFile() {
    if (state.picking) return;
    state.picking = true;
    $("open").disabled = true;
    $("welcome-open").disabled = true;
    try {
      const path = await rpc("pick");
      if (path) await openFile(path);
    } catch (error) {
      notify(error.message, true);
    } finally {
      state.picking = false;
      $("open").disabled = false;
      $("welcome-open").disabled = false;
    }
  }
  async function openFile(path, overrides = {}) {
    cancelColumnDrag();
    clearTimeout(searchTimer);
    ++state.queryVersion;
    const request = rpc("open", {
      path,
      options: { ...settings, ...overrides },
    });
    busy(request, "open", t("Открываем файл…"));
    try {
      const data = await request;
      if (state.busy !== request.id) return;
      state.file = data;
      $("notice").hidden = true;
      state.document = null;
      state.documentOffsets = [0];
      state.documentLines = [1];
      state.documentPage = 0;
      ++state.documentVersion;
      showTable();
      $("document-toggle").hidden = ![
        "text",
        "json",
        "jsonl",
        "xml",
        "markdown",
        "javascript",
        "python",
        "powershell",
        "shell",
        "yaml",
      ].includes(data.kind);
      $("document-mode").value =
        data.kind === "markdown"
          ? "preview"
          : data.kind === "json"
            ? "tree"
            : "source";
      $("document-raw").checked = false;
      $("binary-toggle").hidden = !["binary", "hex"].includes(data.kind);
      $("binary-toggle").textContent = t(
        data.kind === "hex" ? "Читаемый вид" : "Исходные байты",
      );
      state.revision = data.revision;
      state.total = data.total;
      state.rows = data.rows;
      state.headers = displayHeaders(data);
      state.types = data.types;
      state.visible = data.headers.map(
        (name) =>
          !(
            ["ese", "sqlite"].includes(data.kind) && name.startsWith("$raw/")
          ) && !(data.kind === "registry" && name === "Raw data"),
      );
      state.columnOrder = data.headers.map((_, i) => i);
      $("table-header").dataset.key = "";
      state.widths = state.headers.map((name, i) =>
        i === 0
          ? 255
          : /path|details|text|путь|описание/i.test(name)
            ? 310
            : data.types[i] === "date" || data.types[i] === "timestamp"
              ? 215
              : data.types[i] === "number"
                ? 125
                : 175,
      );
      measuredText.clear();
      state.contentWidths = state.headers.map(
        (name, i) => textWidth(name, 600) + (i === 0 ? 66 : 104),
      );
      state.manualWidths = [];
      state.hoverColumn = null;
      state.page = 0;
      state.filters = [];
      state.sort = null;
      state.text = "";
      state.selected = null;
      state.anchor = null;
      state.applied = { filters: [], sort: null, text: "" };
      ++state.inspectorVersion;
      $("search").value = "";
      document.body.classList.add("has-file");
      $("welcome").hidden = true;
      $("workspace").hidden = false;
      $("inspector").hidden = true;
      invalidate();
      viewport.scrollTop = 0;
      viewport.scrollLeft = 0;
      updateControls();
      await primeRows();
      if (data.warnings?.length)
        notify(data.warnings.map(i18n.diagnostic).join("\n"), true);
      if (
        [
          "text",
          "json",
          "markdown",
          "javascript",
          "python",
          "powershell",
          "shell",
          "yaml",
        ].includes(data.kind)
      )
        await showDocument();
      if (data.irregular)
        notify(
          t(
            "{count} строк имеют разное число полей. Отсутствующие значения показаны пустыми.",
            { count: fmt(data.irregular) },
          ),
        );
    } catch (error) {
      if (!error.cancelled && state.busy === request.id) {
        if (error.needsElevation) {
          showDialog(
            t("Нужны права администратора"),
            t(
              "Windows ограничила доступ к этому файлу. Откройте отдельное окно Timeline с правами администратора и подтвердите запрос UAC.",
            ),
          );
          action(t("Выбрать другой файл"), () => {
            $("dialog").close();
            pickFile();
          });
          action(t("Отмена"), () => $("dialog").close());
          const elevateButton = action(
            t("Открыть от имени администратора"),
            async () => {
              elevateButton.disabled = true;
              elevateButton.textContent = t("Ожидаем подтверждения UAC…");
              try {
                await rpc("elevate", {
                  path,
                  settings: { ...settings, ...overrides },
                });
                $("dialog").close();
              } catch (e) {
                notify(e.message, true);
              } finally {
                elevateButton.disabled = false;
                elevateButton.textContent = t(
                  "Открыть от имени администратора",
                );
              }
            },
            true,
          );
        } else {
          notify(error.message, true);
          if (!overrides.format && error.canOpenRaw) {
            showDialog(t("Не удалось разобрать файл"), error.message);
            action(t("Отмена"), () => $("dialog").close());
            action(
              t("Открыть как байты"),
              () => {
                $("dialog").close();
                openFile(path, { format: "hex" });
              },
              true,
            );
          }
        }
      }
    } finally {
      finish(request.id);
    }
  }
  async function applyQuery() {
    clearTimeout(searchTimer);
    if (!state.file || ["open", "export"].includes(state.jobKind)) return;
    showTable();
    const version = ++state.queryVersion;
    state.text = $("search").value;
    const request = rpc("query", {
      file: state.file.file,
      query: {
        text: state.text,
        regex: settings.regex,
        search_mode: settings.searchMode,
        case_sensitive: settings.caseSensitive,
        filters: state.filters,
        sort: state.sort,
      },
    });
    busy(request, "query", t("Ищем и сортируем по всему файлу…"));
    updateChips();
    try {
      const result = await request;
      if (version !== state.queryVersion) return;
      state.revision = result.revision;
      state.rows = result.rows;
      state.page = 0;
      state.selected = null;
      state.anchor = null;
      state.applied = JSON.parse(
        JSON.stringify({
          filters: state.filters,
          sort: state.sort,
          text: state.text,
        }),
      );
      ++state.inspectorVersion;
      state.file.elapsed = result.elapsed;
      viewport.scrollTop = 0;
      invalidate();
      updateControls();
      $("inspector").hidden = true;
      await primeRows();
    } catch (error) {
      if (version === state.queryVersion) {
        Object.assign(state, JSON.parse(JSON.stringify(state.applied)));
        $("search").value = state.text;
        updateControls();
        if (!error.cancelled) notify(error.message, true);
      }
    } finally {
      finish(request.id);
    }
  }
  function invalidate() {
    state.readError = false;
    state.cache.clear();
    state.cacheKey = "";
    schedule();
  }
  function pageStart() {
    return settings.pageSize ? state.page * Number(settings.pageSize) : 0;
  }
  function pageCount() {
    return settings.pageSize
      ? Math.min(
          Number(settings.pageSize),
          Math.max(0, state.rows - pageStart()),
        )
      : state.rows;
  }
  function rowHeight() {
    return settings.compact ? 31 : 39;
  }
  function geometry() {
    const count = pageCount(),
      h = rowHeight(),
      height = Math.max(1, viewport.clientHeight);
    const logicalHeight = count * h,
      physicalHeight = Math.min(logicalHeight, 8_000_000);
    const ratio = Math.max(
      1,
      (logicalHeight - height) / Math.max(1, physicalHeight - height),
    );
    const logicalTop = viewport.scrollTop * ratio;
    return {
      count,
      h,
      physicalHeight,
      logicalTop,
      ratio,
      first: Math.max(0, Math.floor(logicalTop / h) - 7),
      last: Math.min(count, Math.ceil((logicalTop + height) / h) + 8),
    };
  }
  function layoutColumns() {
    let x = 0;
    const all = [];
    for (const i of state.columnOrder) {
      if (!state.visible[i] && i !== 0) continue;
      const width = columnWidth(i);
      all.push({ index: i, x, width, position: all.length });
      x += width;
    }
    const left = viewport.scrollLeft,
      right = left + viewport.clientWidth;
    const visible = all
      .filter(
        (c) =>
          (settings.pinFirstColumn && c.position === 0) ||
          (c.x + c.width > left + frozenWidth() - 100 && c.x < right + 250),
      )
      .slice(0, 128);
    return { all, visible, width: x };
  }
  // Keep source column IDs stable: sorting, cached cells and filters use those IDs.
  function moveColumn(column, before) {
    if (before === column) return;
    const order = state.columnOrder.filter((id) => id !== column);
    const destination = before === null ? order.length : order.indexOf(before);
    if (destination < 0) return;
    order.splice(destination, 0, column);
    state.columnOrder = order;
    schedule();
  }
  let columnDrag = null;
  let suppressColumnClickUntil = 0;
  function cancelColumnDrag() {
    columnDrag?.finish(true);
  }
  function startColumnDrag(event, column) {
    if (event.button !== 0 || !event.isPrimary || state.busy || columnDrag)
      return;
    const header = $("table-header");
    const drag = {
      column,
      startX: event.clientX,
      startY: event.clientY,
      x: event.clientX,
      y: event.clientY,
      active: false,
      valid: false,
      before: column,
      frame: 0,
      lastTime: 0,
      finish: null,
    };
    columnDrag = drag;
    let preview, marker;
    function update(time) {
      if (columnDrag !== drag || !drag.active) return;
      const rect = header.getBoundingClientRect();
      const width = viewport.clientWidth;
      const x = drag.x - rect.left;
      drag.valid =
        drag.y >= rect.top - 24 &&
        drag.y <= rect.bottom + 24 &&
        x >= -24 &&
        x <= width + 24;
      const elapsed = Math.min(40, time - (drag.lastTime || time));
      drag.lastTime = time;
      if (drag.valid) {
        const edge = Math.min(48, width / 5);
        const speed =
          x < edge
            ? -Math.min(1, (edge - x) / edge)
            : x > width - edge
              ? Math.min(1, (x - width + edge) / edge)
              : 0;
        viewport.scrollLeft += speed * elapsed * 0.9;
        const columns = layoutColumns().all;
        const targets = columns
          .map((c) => {
            const pinned = settings.pinFirstColumn && c.position === 0;
            const left = pinned ? 0 : c.x - viewport.scrollLeft;
            return {
              ...c,
              left: Math.max(pinned ? 0 : frozenWidth(), left),
              right: Math.min(width, left + c.width),
            };
          })
          .filter((c) => c.right > c.left);
        const target = targets.find((c) => x < (c.left + c.right) / 2);
        const last = targets.at(-1);
        drag.before = target
          ? target.index
          : (columns[(last?.position ?? -1) + 1]?.index ?? null);
        const markerX = target ? target.left : (last?.right ?? 0);
        marker.style.left = Math.max(1, Math.min(width - 2, markerX)) + "px";
      }
      marker.hidden = !drag.valid;
      preview.classList.toggle("invalid", !drag.valid);
      preview.style.left =
        Math.max(
          4,
          Math.min(innerWidth - preview.offsetWidth - 4, drag.x + 14),
        ) + "px";
      preview.style.top =
        Math.max(
          4,
          Math.min(innerHeight - preview.offsetHeight - 4, drag.y + 16),
        ) + "px";
      drag.frame = requestAnimationFrame(update);
    }
    function move(e) {
      if (e.pointerId !== event.pointerId) return;
      drag.x = e.clientX;
      drag.y = e.clientY;
      if (
        !drag.active &&
        Math.hypot(drag.x - drag.startX, drag.y - drag.startY) >= 6
      ) {
        drag.active = true;
        header.setPointerCapture(event.pointerId);
        document.body.classList.add("reordering");
        header
          .querySelector(`[data-column="${column}"]`)
          ?.classList.add("drag-source");
        preview = el("div", "column-drag-preview", state.headers[column]);
        preview.setAttribute("aria-hidden", "true");
        marker = el("div", "column-drop-marker");
        marker.setAttribute("aria-hidden", "true");
        document.body.append(preview);
        grid.append(marker);
        drag.frame = requestAnimationFrame(update);
      }
      if (drag.active) e.preventDefault();
    }
    function finish(cancelled) {
      if (columnDrag !== drag) return;
      if (drag.active && !cancelled) {
        cancelAnimationFrame(drag.frame);
        update(performance.now());
      }
      columnDrag = null;
      cancelAnimationFrame(drag.frame);
      document.removeEventListener("pointermove", move);
      document.removeEventListener("pointerup", up);
      document.removeEventListener("pointercancel", cancel);
      document.removeEventListener("keydown", key, true);
      window.removeEventListener("blur", cancel);
      header.removeEventListener("lostpointercapture", cancel);
      if (header.hasPointerCapture(event.pointerId))
        header.releasePointerCapture(event.pointerId);
      document.body.classList.remove("reordering");
      header.querySelector(".drag-source")?.classList.remove("drag-source");
      preview?.remove();
      marker?.remove();
      if (drag.active) {
        suppressColumnClickUntil = performance.now() + 150;
        if (!cancelled && drag.valid) moveColumn(column, drag.before);
      }
    }
    function up(e) {
      if (e.pointerId !== event.pointerId) return;
      drag.x = e.clientX;
      drag.y = e.clientY;
      finish(false);
    }
    const cancel = () => finish(true);
    function key(e) {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      finish(true);
    }
    drag.finish = finish;
    document.addEventListener("pointermove", move, { passive: false });
    document.addEventListener("pointerup", up);
    document.addEventListener("pointercancel", cancel);
    document.addEventListener("keydown", key, true);
    window.addEventListener("blur", cancel);
    header.addEventListener("lostpointercapture", cancel);
  }
  function schedule() {
    if (!state.frame)
      state.frame = requestAnimationFrame(() => {
        state.frame = 0;
        render();
      });
  }
  function renderHeader(columns) {
    const key =
      columns.map((c) => c.index + ":" + c.width + ":" + c.x).join("|") +
      ":" +
      viewport.scrollLeft +
      ":" +
      frozenWidth() +
      ":" +
      JSON.stringify(state.sort) +
      ":" +
      JSON.stringify(state.filters);
    if ($("table-header").dataset.key === key) return;
    $("table-header").dataset.key = key;
    const fragment = document.createDocumentFragment();
    columns.forEach((column) => {
      const i = column.index,
        sorted = state.sort?.column === i,
        first = column.position === 0,
        pinned = first && settings.pinFirstColumn;
      const cell = el(
        "div",
        "header-cell" +
          (first ? " first" : "") +
          (pinned ? " pinned" : "") +
          (columnDrag?.active && columnDrag.column === i
            ? " drag-source"
            : "") +
          (sorted ? " sorted" : ""),
      );
      cell.style.left = (pinned ? 0 : column.x - viewport.scrollLeft) + "px";
      cell.style.width = column.width + "px";
      cell.dataset.column = i;
      cell.classList.toggle("column-hover", state.hoverColumn === i);
      cell.setAttribute("role", "columnheader");
      cell.setAttribute("aria-colindex", column.position + 1);
      cell.setAttribute(
        "aria-sort",
        sorted ? (state.sort.descending ? "descending" : "ascending") : "none",
      );
      const button = el("button", "sort-button");
      button.type = "button";
      button.title = t(
        "{name} · {type}. Нажмите для сортировки, потяните для перемещения",
        { name: state.headers[i], type: t(typeLabels[state.types[i]]) },
      );
      button.onpointerdown = (event) => startColumnDrag(event, i);
      button.ondragstart = (event) => event.preventDefault();
      button.setAttribute(
        "aria-label",
        t("Сортировать: {name}", { name: state.headers[i] }),
      );
      const type = el(
        "span",
        "type-icon",
        i === 0
          ? t("Название")
          : state.types[i] === "number"
            ? "#"
            : state.types[i] === "text"
              ? "Aa"
              : "◷",
      );
      if (i !== 0) button.append(type);
      button.append(
        el(
          "span",
          "column-name",
          state.headers[i] || t("Столбец {number}", { number: i + 1 }),
        ),
      );
      const arrow = el("span", "sort-icon");
      arrow.innerHTML = svg(
        sorted ? (state.sort.descending ? "down" : "up") : "sort",
      );
      button.append(arrow);
      button.onclick = () => {
        state.sort =
          sorted && state.sort.descending
            ? null
            : {
                column: i,
                descending: sorted,
                kind: state.sort?.column === i ? state.sort.kind : null,
              };
        applyQuery();
        schedule();
      };
      cell.append(button);
      if (first) {
        const pin = el("button", "icon-button column-menu pin-icon");
        pin.type = "button";
        pin.innerHTML = svg("pin");
        pin.title = pinned
          ? t("Открепить первый столбец")
          : t("Закрепить первый столбец");
        pin.setAttribute("aria-label", pin.title);
        pin.setAttribute("aria-pressed", String(pinned));
        pin.onclick = () => {
          settings.pinFirstColumn = !settings.pinFirstColumn;
          saveSettings();
          schedule();
        };
        cell.append(pin);
      }
      if (i !== 0) {
        const menu = el(
          "button",
          "icon-button column-menu" +
            (state.filters.some((f) => f.column === i) ? " filtered" : ""),
        );
        menu.innerHTML = svg("filter");
        menu.title = t("Настроить столбец {name}", { name: state.headers[i] });
        menu.setAttribute("aria-label", menu.title);
        menu.onclick = () => columnDialog(i);
        cell.append(menu);
      }
      const resize = el("div", "resize-handle");
      resize.title = t(
        "Потяните, чтобы изменить ширину. Двойной щелчок — автоподбор",
      );
      resize.ondblclick = (event) => {
        event.preventDefault();
        event.stopPropagation();
        delete state.manualWidths[i];
        schedule();
      };
      resize.onpointerdown = (event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        event.stopPropagation();
        const start = event.clientX,
          width = columnWidth(i);
        document.body.style.cursor = "col-resize";
        document.body.classList.add("resizing");
        const move = (e) => {
          state.manualWidths[i] = Math.max(
            first ? 120 : 90,
            Math.min(
              first && settings.pinFirstColumn
                ? Math.max(120, viewport.clientWidth - 120)
                : 32000,
              width + e.clientX - start,
            ),
          );
          schedule();
        };
        const end = () => {
          document.removeEventListener("pointermove", move);
          document.removeEventListener("pointerup", end);
          document.removeEventListener("pointercancel", end);
          window.removeEventListener("blur", end);
          document.body.style.cursor = "";
          document.body.classList.remove("resizing");
        };
        document.addEventListener("pointermove", move);
        document.addEventListener("pointerup", end, { once: true });
        document.addEventListener("pointercancel", end, { once: true });
        window.addEventListener("blur", end, { once: true });
      };
      cell.append(resize);
      fragment.append(cell);
    });
    $("table-header").replaceChildren(fragment);
  }
  function appendValue(parent, value) {
    if (!value) {
      parent.append(el("span", "empty-value", "—"));
      return;
    }
    if (
      !state.text ||
      (settings.searchMode !== "text" &&
        (settings.searchMode === "regex" ||
          /^(?:ext:|regex:|\(\?)/i.test(state.text)))
    ) {
      parent.textContent = value;
      return;
    }
    const haystack = settings.caseSensitive ? value : value.toLocaleLowerCase(),
      needle = settings.caseSensitive
        ? state.text
        : state.text.toLocaleLowerCase();
    let from = 0,
      at = haystack.indexOf(needle);
    // DOM text nodes only: imported cells can never become HTML or script.
    for (let n = 0; at >= 0 && n < 24; n++) {
      parent.append(
        document.createTextNode(value.slice(from, at)),
        el("mark", "", value.slice(at, at + needle.length)),
      );
      from = at + needle.length;
      at = haystack.indexOf(needle, from);
    }
    parent.append(document.createTextNode(value.slice(from)));
  }
  function render() {
    if (!state.file || !$("document-pane").hidden) return;
    const g = geometry(),
      layout = layoutColumns(),
      columns = layout.visible,
      ids = columns.map((c) => c.index);
    const cacheKey = String(state.revision);
    if (state.cacheKey !== cacheKey) {
      state.cache.clear();
      state.cacheKey = cacheKey;
    }
    canvas.style.height = g.physicalHeight + "px";
    canvas.style.width = Math.max(layout.width, viewport.clientWidth) + "px";
    document.documentElement.style.setProperty("--row-height", g.h + "px");
    grid.setAttribute("aria-rowcount", state.rows + 1);
    grid.setAttribute("aria-colcount", layout.all.length);
    renderHeader(columns);
    const fragment = document.createDocumentFragment();
    const selection = selectionBounds();
    const selected = (index, column) =>
      selection &&
      index >= selection.start &&
      index <= selection.end &&
      selection.columns.includes(column);
    let missing = -1;
    for (let i = g.first; i < g.last; i++) {
      const index = pageStart() + i,
        cached = state.cache.get(index);
      if (missing < 0 && (!cached || ids.some((c) => !(c in cached.cells))))
        missing = index;
      const row = el(
        "div",
        "data-row" + (state.selected?.index === index ? " selected-row" : ""),
      );
      row.classList.toggle("alternate", index % 2 === 1);
      row.setAttribute("role", "row");
      row.setAttribute("aria-rowindex", index + 2);
      row.style.top = i * g.h - g.logicalTop + viewport.scrollTop + "px";
      row.style.width = Math.max(layout.width, viewport.clientWidth) + "px";
      columns.forEach((column) => {
        const cell = el(
          "div",
          "cell" +
            (column.position === 0 ? " first" : "") +
            (column.position === 0 && settings.pinFirstColumn
              ? " pinned"
              : "") +
            (state.types[column.index] === "number" ? " numeric" : "") +
            (selected(index, column.index) ? " selected" : ""),
        );
        cell.style.left =
          (column.position === 0 && settings.pinFirstColumn ? 0 : column.x) +
          "px";
        cell.style.width = column.width + "px";
        cell.dataset.column = column.index;
        cell.dataset.index = index;
        cell.setAttribute(
          "aria-selected",
          String(!!selected(index, column.index)),
        );
        cell.classList.toggle(
          "column-hover",
          state.hoverColumn === column.index,
        );
        cell.setAttribute("role", "gridcell");
        cell.setAttribute("aria-colindex", column.position + 1);
        cell.id = `cell-${index}-${column.index}`;
        if (column.position === 0)
          cell.append(
            el("span", "row-number", fmt(cached ? cached.id + 1 : index + 1)),
          );
        if (cached && column.index in cached.cells) {
          const text = el("span", "value");
          const value = displayCell(
            cached.cells[column.index] || "",
            column.index,
            cached,
          );
          appendValue(text, value);
          cell.append(text);
          cell.title = value;
        } else cell.append(el("span", "skeleton"));
        row.append(cell);
      });
      fragment.append(row);
    }
    canvas.replaceChildren(fragment);
    if (state.selected)
      grid.setAttribute(
        "aria-activedescendant",
        `cell-${state.selected.index}-${state.selected.column}`,
      );
    else grid.removeAttribute("aria-activedescendant");
    $("copy-selection").disabled = !selection || !!state.busy;
    $("range").textContent = state.rows
      ? t("{start}–{end} из {total}", {
          start: fmt(
            pageStart() +
              Math.min(g.count - 1, Math.floor(g.logicalTop / g.h)) +
              1,
          ),
          end: fmt(
            pageStart() +
              Math.min(
                g.count,
                Math.ceil((g.logicalTop + viewport.clientHeight) / g.h),
              ),
          ),
          total: fmt(state.rows),
        })
      : t("0 строк");
    if (!state.busy && !state.readError) {
      if (missing >= 0)
        loadRows(Math.floor(missing / 256) * 256, ids, cacheKey);
      const block = Math.floor((pageStart() + g.first) / 256) * 256;
      for (const start of [
        block + 256,
        block + 512,
        block - 256,
        block + 768,
      ]) {
        if (start < 0 || start >= state.rows) continue;
        const cached = state.cache.get(start);
        if (!cached || ids.some((c) => !(c in cached.cells)))
          loadRows(start, ids, cacheKey);
      }
    }
  }
  async function loadRows(start, columns, key) {
    const loadKey = key + ":" + start + ":" + columns.join(",");
    if (state.loads.has(loadKey) || state.loads.size >= 3) return;
    state.loads.add(loadKey);
    try {
      const result = await rpc("rows", {
        revision: Number(key),
        start,
        count: Math.min(256, state.rows - start),
        columns,
      });
      if (key !== state.cacheKey) return;
      result.rows.forEach((row, i) => {
        const cached = state.cache.get(result.start + i) || {
          id: row.id,
          cells: {},
        };
        columns.forEach(
          (column, c) => (cached.cells[column] = row.values[c] || ""),
        );
        state.cache.set(result.start + i, cached);
        fitRow(cached, columns);
      });
      if (state.cache.size > 8192) {
        const from = pageStart() + geometry().first;
        for (const index of state.cache.keys())
          if (index < from - 2048 || index > from + 4096)
            state.cache.delete(index);
      }
    } catch (error) {
      if (key === state.cacheKey && !state.busy) {
        state.readError = true;
        notify(error.message, true);
      }
    } finally {
      state.loads.delete(loadKey);
      schedule();
    }
  }
  async function primeRows() {
    state.cacheKey = String(state.revision);
    await loadRows(
      0,
      layoutColumns().visible.map((c) => c.index),
      state.cacheKey,
    );
  }
  function updateControls() {
    $("search-mode").value = settings.searchMode;
    $("page-size").value = settings.pageSize;
    const pages = settings.pageSize
      ? Math.max(1, Math.ceil(state.rows / settings.pageSize))
      : 1;
    state.page = Math.min(state.page, pages - 1);
    $("page-controls").hidden = !settings.pageSize;
    $("page-label").textContent = `${fmt(state.page + 1)} / ${fmt(pages)}`;
    $("previous").disabled = state.page === 0;
    $("next").disabled = state.page >= pages - 1;
    $("view-summary").textContent =
      state.rows === state.total
        ? t("{count} строк", { count: fmt(state.rows) })
        : t("Найдено {count} из {total} строк", {
            count: fmt(state.rows),
            total: fmt(state.total),
          });
    $("no-results").hidden = state.rows !== 0;
    $("no-results").querySelector("h2").textContent =
      state.total === 0 ? t("В файле пока нет строк") : t("Ничего не найдено");
    updateChips();
    schedule();
  }
  function updateChips() {
    $("clear-search").hidden = !$("search").value;
    $("search-shortcut").hidden = !!$("search").value;
    $("filter-count").hidden = !state.filters.length;
    $("filter-count").textContent = state.filters.length;
    const chips = $("chips");
    chips.replaceChildren();
    function chip(text, remove, icon) {
      const node = el("div", "chip");
      if (icon) node.insertAdjacentHTML("afterbegin", svg(icon));
      node.append(el("span", "", text));
      const close = el("button");
      close.innerHTML = svg("close");
      close.setAttribute("aria-label", t("Убрать: {text}", { text }));
      close.onclick = remove;
      node.append(close);
      chips.append(node);
    }
    if (state.text)
      chip(
        t("Поиск: {text}", { text: state.text }),
        () => {
          $("search").value = "";
          applyQuery();
        },
        "search",
      );
    state.filters.forEach((f, i) =>
      chip(
        `${state.headers[f.column]} ${t(ops[f.op]).toLowerCase()} ${f.text || ""}`,
        () => {
          state.filters.splice(i, 1);
          applyQuery();
        },
        "filter",
      ),
    );
    if (state.sort)
      chip(
        `${state.headers[state.sort.column]} · ${state.sort.descending ? t("по убыванию") : t("по возрастанию")}`,
        () => {
          state.sort = null;
          applyQuery();
        },
        state.sort.descending ? "down" : "up",
      );
    if (chips.children.length) {
      const clear = el("button", "text-button", t("Сбросить всё"));
      clear.onclick = reset;
      chips.append(clear);
    }
    chips.hidden = !chips.children.length;
  }
  function reset() {
    state.filters = [];
    state.sort = null;
    $("search").value = "";
    applyQuery();
  }
  function setPageSize(value) {
    settings.pageSize = Number(value);
    state.page = 0;
    viewport.scrollTop = 0;
    saveSettings();
    updateControls();
  }
  function scrollToIndex(index) {
    if (settings.pageSize) {
      state.page = Math.floor(index / settings.pageSize);
      updateControls();
    }
    const g = geometry(),
      local = index - pageStart(),
      top = local * g.h;
    viewport.scrollTop = Math.max(0, top - viewport.clientHeight / 2) / g.ratio;
    schedule();
  }
  async function cellValue(index, column) {
    const cached = state.cache.get(index);
    if (!cached) throw new Error(t("Дождитесь загрузки строки"));
    return rpc("cell", { revision: state.revision, row: cached.id, column });
  }
  function selectionBounds() {
    if (!state.selected) return null;
    const anchor = state.anchor || state.selected;
    const columns = layoutColumns().all.map((c) => c.index);
    const a = columns.indexOf(anchor.column),
      b = columns.indexOf(state.selected.column);
    if (a < 0 || b < 0) return null;
    return {
      start: Math.min(anchor.index, state.selected.index),
      end: Math.max(anchor.index, state.selected.index),
      columns: columns.slice(Math.min(a, b), Math.max(a, b) + 1),
    };
  }
  function selectCell(index, column, extend = false) {
    if (!extend || !state.anchor) state.anchor = { index, column };
    state.selected = { index, column };
    schedule();
  }
  async function copySelection() {
    if (state.busy || !$("document-pane").hidden) return;
    const area = selectionBounds();
    if (!area) return;
    try {
      const result = await rpc("copy_range", {
        revision: state.revision,
        ...area,
      });
      await copy(result.text);
    } catch (error) {
      notify(error.message, true);
    }
  }
  function showTable() {
    $("document-pane").hidden = true;
    $("table-pane").hidden = false;
    $("document-toggle").textContent = t("Просмотр");
    $("copy-selection").hidden = false;
    ++state.documentVersion;
    window.timelineViewer.destroy();
    schedule();
  }
  async function showDocument() {
    if (!state.file) return;
    const version = ++state.documentVersion;
    $("document-pane").hidden = false;
    $("table-pane").hidden = true;
    $("document-toggle").textContent = t("Таблица");
    $("copy-selection").hidden = true;
    $("document-mode").hidden = state.file.kind === "text";
    $("document-raw-label").hidden = state.file.kind === "text";
    $("document-wrap").checked = settings.documentWrap;
    $("document-mode").disabled = $("document-raw").checked;
    for (const option of $("document-mode").options) {
      option.hidden =
        (option.value === "tree" &&
          !["json", "jsonl"].includes(state.file.kind)) ||
        (option.value === "preview" && state.file.kind !== "markdown");
    }
    $("document-status").hidden = true;
    $("document-prev").disabled = true;
    $("document-next").disabled = true;
    $("document-copy").disabled = true;
    $("document-search").disabled = true;
    window.timelineViewer.destroy();
    $("document-content").textContent = t("Загрузка…");
    try {
      const result = await rpc("source", {
        file: state.file.file,
        offset: state.documentOffsets[state.documentPage],
        raw: true,
      });
      if (version !== state.documentVersion) return;
      state.document = result;
      state.documentOffsets[state.documentPage + 1] = result.next;
      state.documentLines[state.documentPage + 1] =
        state.documentLines[state.documentPage] +
        (result.newline_count ?? (result.text.match(/\n/g) || []).length);
      const raw = $("document-raw").checked;
      const mode = raw ? "raw" : $("document-mode").value;
      const whole = result.whole ?? (result.offset === 0 && result.eof);
      const text =
        !raw && mode === "source" && result.kind === "json" && whole
          ? window.timelineViewer.pretty(result.text)
          : result.text;
      const rendered = window.timelineViewer.render(
        $("document-content"),
        text,
        result.kind,
        mode,
        state.documentLines[state.documentPage],
        settings.documentWrap,
      );
      $("document-wrap").disabled = !window.timelineViewer.hasEditor();
      $("document-copy").disabled = false;
      $("document-search").disabled = false;
      $("document-status").textContent = rendered?.notice || "";
      $("document-status").hidden = !rendered?.notice;
      $("document-prev").disabled = state.documentPage === 0;
      $("document-next").disabled = result.eof;
      $("document-prev").hidden = whole;
      $("document-next").hidden = whole;
      $("document-range").textContent =
        result.offset || !result.eof
          ? t("Фрагмент {number} · поиск по всему файлу — в таблице", {
              number: state.documentPage + 1,
            })
          : t("Строк: {count}", {
              count: fmt(
                (result.newline_count ??
                  (result.text.match(/\n/g) || []).length) + 1,
              ),
            });
    } catch (error) {
      if (version === state.documentVersion)
        $("document-content").textContent = error.message;
    }
  }
  async function searchDocument() {
    if (!state.document) return;
    if (!window.timelineViewer.hasEditor()) {
      $("document-mode").value = "source";
      await showDocument();
    }
    window.timelineViewer.search();
  }
  function selectedDocumentText() {
    const selection = window.getSelection();
    return (
      window.timelineViewer.selectedText() ||
      (selection?.rangeCount &&
      $("document-content").contains(selection.anchorNode)
        ? selection.toString()
        : "")
    );
  }
  async function inspect(index, column) {
    selectCell(index, column);
    const version = ++state.inspectorVersion;
    $("inspector").hidden = false;
    $("cell-value").textContent = t("Загрузка…");
    state.cell = null;
    $("cell-mode").hidden = true;
    $("cell-mode").value = "decoded";
    $("cell-warning").hidden = true;
    $("copy-cell").disabled = true;
    $("cell-label").textContent = t("{name} · строка {number}", {
      name: state.headers[column],
      number: fmt((state.cache.get(index)?.id ?? index) + 1),
    });
    schedule();
    try {
      const result = await cellValue(index, column);
      if (version !== state.inspectorVersion) return;
      state.cell = {
        ...result,
        value: displayCell(result.value, column, state.cache.get(index)),
      };
      $("cell-mode").hidden = !result.decoded;
      showCellValue();
      $("cell-warning").hidden = !result.truncated;
      $("copy-cell").disabled = false;
    } catch (error) {
      if (version === state.inspectorVersion)
        $("cell-value").textContent = error.message;
    }
  }
  function showCellValue() {
    if (!state.cell) return;
    const decoded = state.cell.decoded && $("cell-mode").value === "decoded";
    $("cell-value").textContent = decoded
      ? state.cell.decoded
      : state.cell.decoded
        ? state.cell.value
        : window.timelineViewer.pretty(state.cell.value);
  }
  async function copy(text) {
    try {
      if (navigator.clipboard?.writeText)
        await navigator.clipboard.writeText(text);
      else throw new Error();
    } catch {
      const area = el("textarea");
      area.value = text;
      area.style.cssText = "position:fixed;left:-9999px";
      document.body.append(area);
      area.select();
      const success = document.execCommand("copy");
      area.remove();
      grid.focus();
      if (!success) {
        notify(
          t(
            "Не удалось скопировать. Выделите текст в панели и нажмите Ctrl+C.",
          ),
          true,
        );
        return;
      }
    }
    notify(t("Скопировано"));
  }
  function showDialog(title, description, width = 570) {
    $("dialog-title").textContent = title;
    $("dialog-description").textContent = description;
    $("dialog-content").replaceChildren();
    $("dialog-actions").replaceChildren();
    $("dialog").style.width = width + "px";
    if (!$("dialog").open) $("dialog").showModal();
  }
  function action(text, handler, primary = false) {
    const button = el("button", "button" + (primary ? " primary" : ""), text);
    button.onclick = handler;
    $("dialog-actions").append(button);
    return button;
  }
  function select(options, value) {
    const node = el("select");
    for (const [key, label] of Object.entries(options)) {
      const option = el("option", "", label);
      option.value = key;
      node.append(option);
    }
    node.value = value;
    return node;
  }
  function settingRow(title, subtitle, control) {
    const row = el("div", "setting-row"),
      label = el("label", "", title);
    const id = "setting-" + ++sequence;
    control.id = id;
    label.htmlFor = id;
    if (subtitle) label.append(el("small", "", subtitle));
    row.append(label, control);
    $("dialog-content").append(row);
    return control;
  }
  function checkbox(text, value, callback) {
    const label = el("label", "check-row"),
      input = el("input");
    input.type = "checkbox";
    input.checked = value;
    input.onchange = () => callback(input.checked);
    label.append(input, el("span", "", text));
    return label;
  }
  function settingsDialog() {
    showDialog(
      t("Настройки"),
      t("Настройте отображение и параметры открытия файлов."),
    );
    const draft = { ...settings };
    const language = settingRow(
      t("Язык интерфейса"),
      t("Применяется сразу после сохранения"),
      select(
        {
          auto: t("Автоматически (регион системы)"),
          ru: "Русский",
          en: "English",
        },
        draft.language,
      ),
    );
    language.onchange = () => (draft.language = language.value);
    const pages = settingRow(
      t("Строк на странице"),
      t("«Все строки» — одна непрерывная таблица"),
      select(
        {
          0: t("Все строки"),
          100: "100",
          500: "500",
          1000: fmt(1000),
          5000: fmt(5000),
        },
        draft.pageSize,
      ),
    );
    pages.onchange = () => (draft.pageSize = Number(pages.value));
    const density = settingRow(
      t("Высота строк"),
      t("Выберите удобную плотность"),
      select(
        { normal: t("Обычная"), compact: t("Компактная") },
        draft.compact ? "compact" : "normal",
      ),
    );
    density.onchange = () => (draft.compact = density.value === "compact");
    $("dialog-content").append(
      checkbox(
        t("Все колонки в полную длину"),
        draft.fullWidthColumns,
        (v) => (draft.fullWidthColumns = v),
      ),
      checkbox(
        t("Закрепить первый столбец"),
        draft.pinFirstColumn,
        (v) => (draft.pinFirstColumn = v),
      ),
    );
    $("dialog-content").append(
      el("h3", "setting-section", t("Поиск по всем строкам")),
    );
    const checks = el("div", "settings-checks");
    checks.append(
      checkbox(
        t("Учитывать регистр"),
        draft.caseSensitive,
        (v) => (draft.caseSensitive = v),
      ),
    );
    $("dialog-content").append(checks);
    const searchMode = settingRow(
      t("Режим поиска"),
      t(
        "Авто распознаёт regex:, ext: и (?i). Остальной текст ищется буквально.",
      ),
      select(
        { auto: t("Авто"), text: t("Текст"), regex: "Regex" },
        draft.searchMode,
      ),
    );
    searchMode.onchange = () => (draft.searchMode = searchMode.value);
    $("dialog-content").append(
      el("h3", "setting-section", t("При следующем открытии файла")),
    );
    const encoding = settingRow(
      t("Кодировка"),
      t("Если вместо текста отображаются неверные символы"),
      select(
        {
          auto: t("Автоматически"),
          utf8: "UTF-8",
          utf16le: "UTF-16 LE",
          utf16be: "UTF-16 BE",
          1251: "Windows-1251",
          1252: "Windows-1252",
        },
        draft.encoding,
      ),
    );
    encoding.onchange = () => (draft.encoding = encoding.value);
    const delimiter = settingRow(
      t("Разделитель CSV"),
      t("Обычно определяется автоматически"),
      select(
        {
          "": t("Автоматически"),
          ",": t("Запятая"),
          ";": t("Точка с запятой"),
          "\t": t("Табуляция"),
          "|": t("Вертикальная черта"),
        },
        draft.delimiter,
      ),
    );
    delimiter.onchange = () => (draft.delimiter = delimiter.value);
    const format = settingRow(
      t("Формат файла"),
      t("CSV, JSON или построчный текст"),
      select(
        {
          auto: t("Автоматически"),
          csv: "CSV / TSV",
          json: "JSON / JSONL",
          text: t("Текст / журнал"),
          hex: t("Байты (Hex)"),
        },
        draft.format,
      ),
    );
    format.onchange = () => (draft.format = format.value);
    $("dialog-content").append(
      checkbox(
        t("Распознавать таблицы в TXT автоматически"),
        draft.autoText,
        (v) => (draft.autoText = v),
      ),
      checkbox(
        t("Первая строка файла содержит заголовки"),
        draft.header,
        (v) => (draft.header = v),
      ),
    );
    action(t("Отмена"), () => $("dialog").close());
    action(
      t("Сохранить"),
      () => {
        const searchChanged =
          draft.searchMode !== settings.searchMode ||
          draft.caseSensitive !== settings.caseSensitive;
        const pageSizeChanged = draft.pageSize !== settings.pageSize;
        const languageChanged = draft.language !== settings.language;
        settings = { ...draft, interfaceZoom: settings.interfaceZoom };
        if (settings.fullWidthColumns) {
          for (const row of state.cache.values())
            fitRow(row, Object.keys(row.cells).map(Number));
        }
        saveSettings();
        if (pageSizeChanged) {
          state.page = 0;
          viewport.scrollTop = 0;
        }
        $("dialog").close();
        if (languageChanged) applyLanguage();
        else updateControls();
        if (searchChanged && state.file) applyQuery();
      },
      true,
    );
  }
  function columnsDialog() {
    if (!state.file) return;
    showDialog(
      t("Столбцы"),
      t("Выберите, что показывать. Столбец с названием записи всегда включён."),
      450,
    );
    const search = el("input", "dialog-input");
    search.placeholder = t("Найти столбец…");
    search.setAttribute("aria-label", t("Найти столбец"));
    const list = el("div", "columns-list"),
      draft = [...state.visible];
    function draw() {
      list.replaceChildren();
      state.columnOrder.forEach((i) => {
        const name = state.headers[i];
        if (!name.toLowerCase().includes(search.value.toLowerCase())) return;
        const label = checkbox(name, draft[i], (v) => (draft[i] = v));
        if (i === 0) {
          label.querySelector("input").disabled = true;
          label.append(el("small", "", t("Название")));
        } else label.append(el("small", "", t(typeLabels[state.types[i]])));
        list.append(label);
      });
    }
    search.oninput = draw;
    $("dialog-content").append(search, list);
    draw();
    action(t("Показать все"), () => {
      draft.fill(true);
      draw();
    });
    action(
      t("Применить"),
      () => {
        state.visible = draft;
        state.visible[0] = true;
        if (!selectionBounds()) {
          state.selected = null;
          state.anchor = null;
          ++state.inspectorVersion;
          $("inspector").hidden = true;
        }
        $("dialog").close();
        invalidate();
      },
      true,
    );
  }
  function filterEditor(draft, columns) {
    const list = el("div");
    function draw() {
      list.replaceChildren();
      if (!draft.length)
        list.append(
          el(
            "p",
            "dialog-empty",
            t(
              "Добавьте условие, чтобы оставить только нужные строки.\nВсе условия применяются вместе.",
            ),
          ),
        );
      draft.forEach((f, i) => {
        const row = el("div", "filter-row"),
          col = select(columns, f.column),
          op = select(
            Object.fromEntries(
              Object.entries(ops).map(([key, label]) => [key, t(label)]),
            ),
            f.op,
          ),
          input = el("input");
        col.setAttribute("aria-label", t("Столбец фильтра"));
        op.setAttribute("aria-label", t("Условие фильтра"));
        input.value = f.text;
        input.placeholder = t("Значение");
        input.setAttribute("aria-label", t("Значение фильтра"));
        input.disabled = ["empty", "notEmpty"].includes(f.op);
        col.onchange = () => (f.column = Number(col.value));
        op.onchange = () => {
          f.op = op.value;
          input.disabled = ["empty", "notEmpty"].includes(f.op);
        };
        input.oninput = () => (f.text = input.value);
        const remove = el("button", "icon-button");
        remove.innerHTML = svg("close");
        remove.setAttribute("aria-label", t("Удалить условие"));
        remove.onclick = () => {
          draft.splice(i, 1);
          draw();
        };
        row.append(col, op, input, remove);
        list.append(row);
      });
    }
    draw();
    return { list, draw };
  }
  function filtersDialog() {
    if (!state.file) return;
    showDialog(
      t("Фильтры"),
      t(
        "Условия применяются ко всему файлу, включая строки на других страницах.",
      ),
      690,
    );
    const draft = state.filters.map((f) => ({ ...f })),
      columns = Object.fromEntries(
        state.headers.map((h, i) => [i, h]).filter(([i]) => i > 0),
      );
    const editor = filterEditor(draft, columns);
    $("dialog-content").append(editor.list);
    const add = el("button", "text-button", t("+ Добавить условие"));
    add.disabled = state.headers.length < 2;
    add.onclick = () => {
      draft.push({ column: 1, op: "contains", text: "" });
      editor.draw();
    };
    $("dialog-content").append(add);
    action(t("Сбросить"), () => {
      state.filters = [];
      $("dialog").close();
      applyQuery();
    });
    action(
      t("Применить"),
      () => {
        state.filters = draft.filter(
          (f) => f.text || ["empty", "notEmpty"].includes(f.op),
        );
        $("dialog").close();
        applyQuery();
      },
      true,
    );
  }
  function columnDialog(column) {
    showDialog(
      state.headers[column],
      t("Сортировка и фильтр по всему файлу."),
      530,
    );
    let sort = state.sort?.column === column ? { ...state.sort } : null;
    const sorts = el("div", "sort-actions");
    for (const [desc, label] of [
      [false, t("По возрастанию")],
      [true, t("По убыванию")],
    ]) {
      const button = el(
        "button",
        "button" + (sort?.descending === desc ? " active" : ""),
        label,
      );
      button.insertAdjacentHTML("afterbegin", svg(desc ? "down" : "up"));
      button.onclick = () => {
        sort = { column, descending: desc, kind: kind.value || null };
        sorts
          .querySelectorAll("button")
          .forEach((b) => b.classList.remove("active"));
        button.classList.add("active");
      };
      sorts.append(button);
    }
    $("dialog-content").append(sorts);
    const kind = settingRow(
      t("Сравнивать как"),
      t("Тип определяется по значениям столбца"),
      select(
        {
          "": t("Авто · {type}", { type: t(typeLabels[state.types[column]]) }),
          text: t("Текст"),
          number: t("Число"),
          date: t("Дата и время"),
          timestamp: t("Unix / дата"),
        },
        sort?.kind || "",
      ),
    );
    kind.onchange = () => {
      if (sort) sort.kind = kind.value || null;
    };
    const draft = state.filters
      .filter((f) => f.column === column)
      .map((f) => ({ ...f }));
    const editor = filterEditor(draft, { [column]: state.headers[column] });
    $("dialog-content").append(editor.list);
    const add = el("button", "text-button", t("+ Добавить условие"));
    add.onclick = () => {
      draft.push({ column, op: "contains", text: "" });
      editor.draw();
    };
    $("dialog-content").append(add);
    $("dialog-content").append(
      el(
        "p",
        "dialog-note",
        t(
          "Даты вида 03/04/2026 читаются как 3 апреля. Пустые значения остаются в конце списка.",
        ),
      ),
    );
    action(t("Сбросить"), () => {
      state.filters = state.filters.filter((f) => f.column !== column);
      if (state.sort?.column === column) state.sort = null;
      $("dialog").close();
      applyQuery();
    });
    action(
      t("Применить"),
      () => {
        if (sort) state.sort = sort;
        state.filters = [
          ...state.filters.filter((f) => f.column !== column),
          ...draft.filter(
            (f) => f.text || ["empty", "notEmpty"].includes(f.op),
          ),
        ];
        $("dialog").close();
        applyQuery();
      },
      true,
    );
  }
  $("open").onclick = pickFile;
  $("toolbar-open").onclick = pickFile;
  $("toolbar-settings").onclick = settingsDialog;
  $("welcome-open").onclick = pickFile;
  $("settings").onclick = settingsDialog;
  $("import-settings").onclick = settingsDialog;
  $("filters").onclick = filtersDialog;
  $("columns").onclick = columnsDialog;
  $("search-form").onsubmit = (e) => {
    e.preventDefault();
    applyQuery();
  };
  $("search").oninput = () => {
    clearTimeout(searchTimer);
    updateChips();
    searchTimer = setTimeout(applyQuery, 320);
  };
  $("search-mode").value = settings.searchMode;
  $("search-mode").onchange = () => {
    settings.searchMode = $("search-mode").value;
    saveSettings();
    applyQuery();
  };
  $("document-toggle").onclick = () =>
    $("document-pane").hidden ? showDocument() : showTable();
  $("binary-toggle").onclick = () => {
    if (state.file && !state.busy)
      openFile(state.file.path, {
        format: state.file.kind === "hex" ? "auto" : "hex",
      });
  };
  $("document-mode").onchange = showDocument;
  $("document-raw").onchange = showDocument;
  $("document-wrap").onchange = () => {
    settings.documentWrap = $("document-wrap").checked;
    saveSettings();
    window.timelineViewer.setWrap(settings.documentWrap);
  };
  $("document-search").onclick = searchDocument;
  $("document-copy").onclick = () =>
    copy(selectedDocumentText() || state.document?.text || "");
  $("document-prev").onclick = () => {
    if (state.documentPage > 0) {
      state.documentPage--;
      showDocument();
    }
  };
  $("document-next").onclick = () => {
    if (state.document && !state.document.eof) {
      state.documentPage++;
      showDocument();
    }
  };
  $("copy-selection").onclick = copySelection;
  $("clear-search").onclick = () => {
    $("search").value = "";
    applyQuery();
    $("search").focus();
  };
  $("page-size").onchange = (e) => setPageSize(e.target.value);
  $("previous").onclick = () => {
    if (state.page > 0) {
      state.page--;
      viewport.scrollTop = 0;
      updateControls();
    }
  };
  $("next").onclick = () => {
    if ((state.page + 1) * settings.pageSize < state.rows) {
      state.page++;
      viewport.scrollTop = 0;
      updateControls();
    }
  };
  $("reset-empty").onclick = reset;
  $("notice-close").onclick = () => ($("notice").hidden = true);
  $("cancel").onclick = () =>
    rpc("cancel").catch((error) => notify(error.message, true));
  $("close-inspector").onclick = () => {
    ++state.inspectorVersion;
    $("inspector").hidden = true;
    schedule();
  };
  $("copy-cell").onclick = () => copy($("cell-value").textContent);
  $("cell-mode").onchange = showCellValue;
  $("dialog-close").onclick = () => $("dialog").close();
  $("dialog").addEventListener("click", (e) => {
    if (e.target === $("dialog")) {
      const r = $("dialog").getBoundingClientRect();
      if (
        e.clientX < r.left ||
        e.clientX > r.right ||
        e.clientY < r.top ||
        e.clientY > r.bottom
      )
        $("dialog").close();
    }
  });
  $("export").onclick = async () => {
    if (!state.file || state.busy) return;
    const request = rpc("export", { revision: state.revision });
    busy(request, "export", t("Сохраняем CSV…"));
    try {
      const result = await request;
      if (result)
        notify(
          t("Сохранено {count} строк: {path}", {
            count: fmt(result.rows),
            path: result.path,
          }),
        );
    } catch (error) {
      if (!error.cancelled) notify(error.message, true);
    } finally {
      finish(request.id);
    }
  };
  viewport.addEventListener("scroll", schedule, { passive: true });
  $("table-header").addEventListener(
    "click",
    (event) => {
      if (event.detail && performance.now() < suppressColumnClickUntil) {
        event.preventDefault();
        event.stopPropagation();
      }
    },
    { capture: true },
  );
  grid.addEventListener("pointerover", (event) => {
    const cell = event.target.closest("[data-column]");
    hoverColumn(cell ? Number(cell.dataset.column) : null);
  });
  grid.addEventListener("pointerleave", () => hoverColumn(null));
  let selectionDrag = null,
    selectionFrame = 0,
    lastCellPointer = null;
  canvas.addEventListener("pointerdown", (event) => {
    const cell = event.target.closest(".cell");
    if (!cell || event.button !== 0 || state.busy) return;
    event.preventDefault();
    grid.focus({ preventScroll: true });
    selectCell(
      Number(cell.dataset.index),
      Number(cell.dataset.column),
      event.shiftKey,
    );
    const click = {
      index: Number(cell.dataset.index),
      column: Number(cell.dataset.column),
      time: performance.now(),
      file: state.file.file,
    };
    const doubleClick =
      !event.shiftKey &&
      lastCellPointer &&
      click.file === lastCellPointer.file &&
      click.index === lastCellPointer.index &&
      click.column === lastCellPointer.column &&
      click.time - lastCellPointer.time < 450;
    lastCellPointer = doubleClick ? null : click;
    if (doubleClick) {
      inspect(Number(cell.dataset.index), Number(cell.dataset.column));
      return;
    }
    selectionDrag = {
      x: event.clientX,
      y: event.clientY,
      startX: event.clientX,
      startY: event.clientY,
      moved: false,
      id: event.pointerId,
    };
    grid.setPointerCapture(event.pointerId);
    const tick = () => {
      if (!selectionDrag) return;
      if (!selectionDrag.moved) {
        selectionFrame = requestAnimationFrame(tick);
        return;
      }
      const r = viewport.getBoundingClientRect();
      const g = geometry();
      if (selectionDrag.y < r.top + 22)
        viewport.scrollTop -= rowHeight() / g.ratio;
      if (selectionDrag.y > r.bottom - 22)
        viewport.scrollTop += rowHeight() / g.ratio;
      if (selectionDrag.x < r.left + 22) viewport.scrollLeft -= 18;
      if (selectionDrag.x > r.right - 22) viewport.scrollLeft += 18;
      const next = geometry();
      const index = Math.min(
        pageStart() + next.count - 1,
        Math.max(
          pageStart(),
          pageStart() +
            Math.floor(
              (next.logicalTop +
                Math.max(0, Math.min(r.height - 1, selectionDrag.y - r.top))) /
                next.h,
            ),
        ),
      );
      const x = selectionDrag.x - r.left + viewport.scrollLeft;
      const all = layoutColumns().all;
      const column =
        settings.pinFirstColumn && selectionDrag.x - r.left < frozenWidth()
          ? all[0]
          : all.find((c) => x >= c.x && x < c.x + c.width) || all.at(-1);
      if (column) selectCell(index, column.index, true);
      selectionFrame = requestAnimationFrame(tick);
    };
    selectionFrame = requestAnimationFrame(tick);
  });
  grid.addEventListener("pointermove", (event) => {
    if (selectionDrag) {
      if (
        Math.hypot(
          event.clientX - selectionDrag.startX,
          event.clientY - selectionDrag.startY,
        ) > 3
      ) {
        selectionDrag.moved = true;
        lastCellPointer = null;
      }
      selectionDrag.x = event.clientX;
      selectionDrag.y = event.clientY;
    }
  });
  const endSelection = () => {
    if (selectionDrag && grid.hasPointerCapture(selectionDrag.id))
      grid.releasePointerCapture(selectionDrag.id);
    selectionDrag = null;
    cancelAnimationFrame(selectionFrame);
  };
  grid.addEventListener("pointerup", endSelection);
  grid.addEventListener("pointercancel", endSelection);
  grid.addEventListener("lostpointercapture", endSelection);
  document.addEventListener(
    "keydown",
    (event) => {
      if (!event.ctrlKey || event.altKey || event.metaKey) return;
      const plus =
        ["Equal", "NumpadAdd"].includes(event.code) ||
        ["+", "="].includes(event.key);
      const minus =
        ["Minus", "NumpadSubtract"].includes(event.code) || event.key === "-";
      const reset =
        ["Digit0", "Numpad0"].includes(event.code) || event.key === "0";
      if (!plus && !minus && !reset) return;
      event.preventDefault();
      event.stopPropagation();
      if (reset) setZoom(1);
      else stepZoom(plus ? 1 : -1);
    },
    { capture: true },
  );
  let wheelZoomDelta = 0,
    lastZoomWheel = 0;
  document.addEventListener(
    "wheel",
    (event) => {
      if (!event.ctrlKey) return;
      event.preventDefault();
      event.stopPropagation();
      const now = performance.now();
      if (
        now - lastZoomWheel > 200 ||
        Math.sign(event.deltaY) !== Math.sign(wheelZoomDelta)
      )
        wheelZoomDelta = 0;
      lastZoomWheel = now;
      wheelZoomDelta +=
        event.deltaY *
        (event.deltaMode === 1 ? 40 : event.deltaMode === 2 ? 800 : 1);
      if (Math.abs(wheelZoomDelta) >= 60) {
        stepZoom(wheelZoomDelta < 0 ? 1 : -1);
        wheelZoomDelta = 0;
      }
    },
    { passive: false, capture: true },
  );
  viewport.addEventListener(
    "wheel",
    (e) => {
      const g = geometry();
      if (
        g.ratio > 1 &&
        !e.ctrlKey &&
        Math.abs(e.deltaY) > Math.abs(e.deltaX)
      ) {
        e.preventDefault();
        const pixels =
          e.deltaY *
          (e.deltaMode === 1
            ? rowHeight()
            : e.deltaMode === 2
              ? viewport.clientHeight
              : 1);
        viewport.scrollTop += pixels / g.ratio;
      }
    },
    { passive: false },
  );
  new ResizeObserver(schedule).observe(viewport);
  grid.addEventListener("keydown", async (e) => {
    if (state.busy || !state.rows) return;
    if (e.ctrlKey && e.code === "KeyC" && state.selected) {
      e.preventDefault();
      await copySelection();
      return;
    }
    if (e.ctrlKey && e.code === "KeyA") {
      e.preventDefault();
      const columns = layoutColumns().all;
      state.anchor = { index: 0, column: columns[0].index };
      selectCell(state.rows - 1, columns.at(-1).index, true);
      return;
    }
    if (e.key === "Enter" && state.selected) {
      e.preventDefault();
      inspect(state.selected.index, state.selected.column);
      return;
    }
    if (
      ![
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
        "PageDown",
        "PageUp",
        "Home",
        "End",
      ].includes(e.key)
    )
      return;
    e.preventDefault();
    const columns = layoutColumns().all;
    let index = state.selected?.index ?? pageStart(),
      column = state.selected?.column ?? firstColumn(),
      c = Math.max(
        0,
        columns.findIndex((x) => x.index === column),
      );
    if (e.key === "ArrowUp") index--;
    if (e.key === "ArrowDown") index++;
    if (e.key === "PageDown")
      index += Math.floor(viewport.clientHeight / rowHeight());
    if (e.key === "PageUp")
      index -= Math.floor(viewport.clientHeight / rowHeight());
    if (e.key === "ArrowLeft") c--;
    if (e.key === "ArrowRight") c++;
    if (e.key === "Home") {
      if (e.ctrlKey) index = 0;
      else c = 0;
    }
    if (e.key === "End") {
      if (e.ctrlKey) index = state.rows - 1;
      else c = columns.length - 1;
    }
    index = Math.max(0, Math.min(state.rows - 1, index));
    c = Math.max(0, Math.min(columns.length - 1, c));
    column = columns[c].index;
    selectCell(index, column, e.shiftKey);
    const g = geometry(),
      local = index - pageStart();
    if (
      local * g.h < g.logicalTop ||
      local * g.h + g.h > g.logicalTop + viewport.clientHeight ||
      local >= g.count ||
      local < 0
    )
      scrollToIndex(index);
    const target = columns[c];
    if (c !== 0 || !settings.pinFirstColumn) {
      if (
        target.width > viewport.clientWidth - frozenWidth() ||
        target.x < viewport.scrollLeft + frozenWidth()
      )
        viewport.scrollLeft = target.x - frozenWidth();
      else if (
        target.x + target.width >
        viewport.scrollLeft + viewport.clientWidth
      )
        viewport.scrollLeft = target.x + target.width - viewport.clientWidth;
    }
    schedule();
  });
  document.addEventListener("keydown", (e) => {
    if (e.defaultPrevented) return;
    if (e.ctrlKey && e.code === "KeyO") {
      e.preventDefault();
      pickFile();
    }
    if (e.ctrlKey && e.code === "KeyF" && state.file && !$("dialog").open) {
      e.preventDefault();
      if (!$("document-pane").hidden && e.target !== $("search")) {
        searchDocument();
      } else {
        $("search").focus();
        $("search").select();
      }
    }
    if (
      e.ctrlKey &&
      e.code === "KeyC" &&
      !$("document-pane").hidden &&
      $("document-content").contains(e.target) &&
      !e.target.closest(".cm-search")
    ) {
      const selected = selectedDocumentText();
      if (selected) {
        e.preventDefault();
        copy(selected);
      }
    }
    if (
      e.ctrlKey &&
      !$("document-pane").hidden &&
      !$("dialog").open &&
      ["PageUp", "PageDown"].includes(e.code)
    ) {
      e.preventDefault();
      $(e.code === "PageUp" ? "document-prev" : "document-next").click();
    }
    if (e.key === "Escape" && !$("dialog").open) {
      $("notice").hidden = true;
      $("inspector").hidden = true;
      schedule();
    }
  });
  let ready = false;
  const reveal = () => {
    if (!ready && window.ipc) {
      ready = true;
      window.ipc.postMessage(
        JSON.stringify({ command: "ready", zoom: settings.interfaceZoom }),
      );
    }
  };
  requestAnimationFrame(() => requestAnimationFrame(reveal));
  // Hidden native windows may throttle animation frames. CSS is already parsed here.
  setTimeout(reveal, 120);
})();
