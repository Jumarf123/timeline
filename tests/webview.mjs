// End-to-end tests against the real Rust executable and WebView2, without a mock backend.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { createServer } from "node:net";
import assert from "node:assert/strict";

const root = resolve(import.meta.dirname, ".."),
  artifacts = resolve(root, "test-results");
await mkdir(artifacts, { recursive: true });
const fixture = resolve(artifacts, "large-ui.csv");
const size = 500257;
const lines = ["Название,Создан,PID,Статус,Путь,Описание"];
for (let i = 0; i < size; i++) {
  const date =
    i === size - 1
      ? "31.12.2025 23:59:59"
      : i === 0
        ? "03.04.2027 10:30:00"
        : new Date(Date.UTC(2026, 0, 1, 0, 0, i)).toISOString();
  const name =
    i === size - 1
      ? "последняя-запись"
      : `процесс-${String(i).padStart(6, "0")}`;
  const filePath =
    i === 0
      ? `C:\\evidence\\${"длинный-каталог-".repeat(14)}\\process.exe`
      : `C:\\Windows\\System32\\process-${i}.exe`;
  lines.push(
    `${name},${date},${size - i},${i % 3 ? "Работает" : "Остановлен"},${filePath},${i === size - 1 ? "NEEDLE_LAST" : i === 0 ? "<img src=x onerror=alert(1)>" : "Событие запуска процесса"}`,
  );
}
await writeFile(fixture, lines.join("\n"));
const port = await new Promise((resolvePort) => {
  const server = createServer();
  server.listen(0, "127.0.0.1", () => {
    const port = server.address().port;
    server.close(() => resolvePort(port));
  });
});
const exe =
  process.env.TIMELINE_EXE || resolve(root, "target/debug/timeline.exe");
const app = spawn(exe, [fixture], {
  cwd: root,
  windowsHide: true,
  env: {
    ...process.env,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
    TIMELINE_PROFILE: resolve(artifacts, "webview-profile-" + Date.now()),
    TIMELINE_REGION: "RU",
  },
  stdio: "pipe",
});
let browser, page;
const errors = [];
app.on("error", (error) => errors.push(String(error)));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const start = performance.now();
try {
  for (let retry = 0; retry < 120; retry++) {
    if (app.exitCode !== null)
      throw new Error(`Application exited with ${app.exitCode}`);
    try {
      const response = await fetch(`http://127.0.0.1:${port}/json/version`);
      if (response.ok) {
        browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
        break;
      }
    } catch {}
    await wait(250);
  }
  assert(browser, "Native WebView2 debugger did not become available");
  for (let retry = 0; retry < 40; retry++) {
    page = browser.contexts()[0]?.pages()[0];
    if (page) break;
    await wait(250);
  }
  assert(page, "Native webview page not found");
  page.setDefaultTimeout(20000);
  page.on("pageerror", (error) => errors.push(error.message));
  await page.waitForFunction(
    () =>
      document.querySelector("#workspace") &&
      !document.querySelector("#workspace").hidden &&
      !document.querySelector("#table-busy").hidden === false,
  );
  await page.locator(".cell .value").first().waitFor();
  console.log(
    `Native startup + indexing ${size} rows: ${((performance.now() - start) / 1000).toFixed(2)}s`,
  );
  await page.screenshot({ path: resolve(artifacts, "table.png") });
  // Auto width is enabled on a fresh profile and fits a path wider than 900px.
  const pathFits = () => {
    const value = document.querySelector(
      '.data-row [aria-colindex="5"] .value',
    );
    return value && value.scrollWidth <= value.clientWidth;
  };
  await page.waitForFunction(pathFits);
  // The default is one horizontally scrolling table, including its first column.
  const firstColumnPositions = () =>
    page.evaluate(() => [
      document.querySelector(".cell.first").getBoundingClientRect().x,
      document.querySelector(".header-cell.first").getBoundingClientRect().x,
    ]);
  const unpinnedOrigin = await firstColumnPositions();
  assert.equal(
    await page
      .locator(".cell.first")
      .first()
      .evaluate((cell) => getComputedStyle(cell).position),
    "absolute",
  );
  for (const offset of [80, 170, 0]) {
    await page
      .locator("#viewport")
      .evaluate((v, offset) => (v.scrollLeft = offset), offset);
    await page.waitForFunction(
      (offset) =>
        Math.abs(
          parseFloat(document.querySelector(".header-cell.first").style.left) +
            offset,
        ) < 1,
      offset,
    );
    const positions = await firstColumnPositions();
    positions.forEach((position, i) =>
      assert(
        Math.abs(position - (unpinnedOrigin[i] - offset)) < 1,
        "The first header and cells must scroll together",
      ),
    );
  }
  const headerOrder = () =>
    page
      .locator(".header-cell")
      .evaluateAll((cells) => cells.map((c) => Number(c.dataset.column)));
  const orderStartsWith = (order) =>
    page.waitForFunction((order) => {
      const headers = [...document.querySelectorAll(".header-cell")];
      return order.every((id, i) => Number(headers[i]?.dataset.column) === id);
    }, order);
  async function dragColumn(source, target, side = "before", cancel = false) {
    await page
      .locator(`.header-cell[data-column="${source}"] .sort-button`)
      .waitFor({ state: "visible" });
    await page
      .locator(`.header-cell[data-column="${target}"]`)
      .waitFor({ state: "visible" });
    const [from, to] = await page.evaluate(
      ({ source, target }) => [
        document
          .querySelector(`.header-cell[data-column="${source}"] .sort-button`)
          ?.getBoundingClientRect()
          .toJSON(),
        document
          .querySelector(`.header-cell[data-column="${target}"]`)
          ?.getBoundingClientRect()
          .toJSON(),
      ],
      { source, target },
    );
    assert(from && to);
    await page.mouse.move(
      from.x + Math.min(60, from.width / 2),
      from.y + from.height / 2,
    );
    await page.mouse.down();
    await page.mouse.move(
      to.x + to.width * (side === "before" ? 0.25 : 0.75),
      to.y + to.height / 2,
      { steps: 12 },
    );
    await page.locator(".column-drop-marker:not([hidden])").waitFor();
    if (cancel) await page.keyboard.press("Escape");
    await page.mouse.up();
    await page.waitForTimeout(180);
    assert.equal(
      await page.locator(".column-drag-preview, .column-drop-marker").count(),
      0,
    );
  }
  // Real pointer drags move the second column before the first on a 500k-row file.
  await dragColumn(1, 0);
  await orderStartsWith([1, 0, 2, 3]);
  assert.equal(
    await page
      .locator('.data-row [data-column="1"] .value')
      .first()
      .textContent(),
    "03.04.2027 10:30:00",
  );
  assert.equal(
    await page
      .locator('.data-row [data-column="0"] .value')
      .first()
      .textContent(),
    "процесс-000000",
  );
  assert.equal(
    await page
      .locator(
        '.header-cell[aria-sort="ascending"], .header-cell[aria-sort="descending"]',
      )
      .count(),
    0,
    "Dragging must not trigger sorting",
  );
  await page.locator("#viewport").evaluate((v) => (v.scrollTop = 30000));
  await page.waitForFunction(
    () => document.querySelectorAll(".skeleton").length === 0,
  );
  await orderStartsWith([1, 0, 2, 3]);
  await page.locator("#viewport").evaluate((v) => (v.scrollTop = 0));
  await dragColumn(0, 1);
  await orderStartsWith([0, 1, 2, 3]);
  await page.locator("#toolbar-settings").click();
  assert.equal(
    await page
      .getByRole("checkbox", { name: "Закрепить первый столбец", exact: true })
      .isChecked(),
    false,
  );
  await page
    .getByRole("checkbox", { name: "Закрепить первый столбец", exact: true })
    .check();
  assert(
    await page
      .getByLabel("Все колонки в полную длину", { exact: true })
      .isChecked(),
  );
  await page
    .getByLabel("Все колонки в полную длину", { exact: true })
    .uncheck();
  await page.getByRole("button", { name: "Сохранить", exact: true }).click();
  await page.waitForFunction(() => {
    const value = document.querySelector(
      '.data-row [aria-colindex="5"] .value',
    );
    return value && value.scrollWidth > value.clientWidth;
  });
  await page.locator("#toolbar-settings").click();
  assert.equal(
    await page
      .getByLabel("Все колонки в полную длину", { exact: true })
      .isChecked(),
    false,
  );
  await page.getByLabel("Все колонки в полную длину", { exact: true }).check();
  await page.getByRole("button", { name: "Сохранить", exact: true }).click();
  await page.waitForFunction(pathFits);
  assert.equal(
    await page.evaluate(
      () =>
        JSON.parse(localStorage.getItem("timeline.settings")).fullWidthColumns,
    ),
    true,
  );
  const headerWidth = (column) =>
    page
      .locator(`.header-cell[data-column="${column}"]`)
      .evaluate((cell) => cell.getBoundingClientRect().width);
  async function resizeColumn(column, delta) {
    const handle = page.locator(
      `.header-cell[data-column="${column}"] .resize-handle`,
    );
    await handle.waitFor({ state: "visible" });
    const box = await page.evaluate((column) => {
      const handle = document.querySelector(
        `.header-cell[data-column="${column}"] .resize-handle`,
      );
      return handle?.getBoundingClientRect().toJSON();
    }, column);
    assert(box, "Column separator must be reachable");
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(
      box.x + box.width / 2 + delta,
      box.y + box.height / 2,
      { steps: 8 },
    );
    await page.mouse.up();
    await page.waitForTimeout(100);
  }
  // Manual sizing must override auto widths, including after a new cache block arrives.
  const autoWidth = await headerWidth(1);
  await resizeColumn(1, -100);
  assert(Math.abs((await headerWidth(1)) - (autoWidth - 100)) < 2);
  await page.locator("#viewport").evaluate((v) => (v.scrollTop = 30000));
  await page.waitForFunction(
    () => document.querySelectorAll(".skeleton").length === 0,
  );
  assert(
    Math.abs((await headerWidth(1)) - (autoWidth - 100)) < 2,
    "Prefetch must not undo a manual width",
  );
  await resizeColumn(1, 160);
  assert(Math.abs((await headerWidth(1)) - (autoWidth + 60)) < 2);
  await page.locator('.header-cell[data-column="1"] .resize-handle').dblclick();
  await page.waitForFunction(
    (width) =>
      Math.abs(
        document
          .querySelector('.header-cell[data-column="1"]')
          .getBoundingClientRect().width - width,
      ) < 2,
    autoWidth,
  );
  await page.locator("#viewport").evaluate((v) => (v.scrollTop = 0));
  await page.locator('.header-cell[data-column="1"] .column-name').hover();
  assert.equal(
    await page
      .locator('.header-cell[data-column="1"]')
      .evaluate((cell) => getComputedStyle(cell).backgroundColor),
    "rgb(209, 229, 219)",
  );
  assert(
    await page
      .locator('.cell[data-column="1"]')
      .first()
      .evaluate((cell) => cell.classList.contains("column-hover")),
  );
  await page.locator('.cell[data-column="1"] .value').first().hover();
  assert.equal(
    await page
      .locator('.cell[data-column="1"]')
      .first()
      .evaluate((cell) => getComputedStyle(cell).backgroundColor),
    "rgb(198, 223, 210)",
  );
  await page.locator("#search").hover();
  assert.equal(await page.locator(".column-hover").count(), 0);

  // Verify the real WebView zoom factor through pixel ratio, not just saved JS settings.
  const basePixelRatio = await page.evaluate(() => devicePixelRatio);
  const zoomed = (factor) =>
    page.waitForFunction(
      ({ base, factor }) =>
        Math.abs(devicePixelRatio / base - factor) < 0.015 &&
        JSON.parse(localStorage.getItem("timeline.settings")).interfaceZoom ===
          factor,
      { base: basePixelRatio, factor },
    );
  await page.locator("#search").focus();
  await page.keyboard.press("Control+Equal");
  await zoomed(1.1);
  await page.keyboard.press("Control+Shift+Equal");
  await zoomed(1.25);
  await page.keyboard.press("Control+Minus");
  await zoomed(1.1);
  await page.keyboard.press("Control+0");
  await zoomed(1);
  for (const [key, code, factor] of [
    ["ъ", "Equal", 1.1],
    ["ß", "Minus", 1],
    ["+", "NumpadAdd", 1.1],
    ["-", "NumpadSubtract", 1],
    ["+", "BracketRight", 1.1],
    ["à", "Digit0", 1],
  ]) {
    await page.evaluate(
      ({ key, code }) =>
        document.dispatchEvent(
          new KeyboardEvent("keydown", {
            key,
            code,
            ctrlKey: true,
            bubbles: true,
            cancelable: true,
          }),
        ),
      { key, code },
    );
    await zoomed(factor);
  }
  await page.locator("#viewport").hover();
  const scrollBeforeZoom = await page
    .locator("#viewport")
    .evaluate((v) => v.scrollTop);
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -120);
  await page.keyboard.up("Control");
  await zoomed(1.1);
  assert.equal(
    await page.locator("#viewport").evaluate((v) => v.scrollTop),
    scrollBeforeZoom,
  );
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, 120);
  await page.keyboard.up("Control");
  await zoomed(1);
  await page.locator("#toolbar-settings").click();
  await page.keyboard.press("Control+Equal");
  await zoomed(1.1);
  await page.getByRole("button", { name: "Сохранить", exact: true }).click();
  await zoomed(1.1);
  await page.keyboard.press("Control+0");
  await zoomed(1);
  assert.equal(
    await page.evaluate(
      () =>
        document.querySelector("#viewport").clientHeight > innerHeight * 0.7,
    ),
    true,
  );
  assert.equal(
    await page
      .getByText("Рабочее пространство для таблиц", { exact: true })
      .count(),
    0,
  );
  await page.evaluate(() =>
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "а",
        code: "KeyF",
        ctrlKey: true,
        bubbles: true,
      }),
    ),
  );
  assert.equal(await page.evaluate(() => document.activeElement.id), "search");
  await page.locator("#data-grid").focus();
  await page.waitForFunction(
    () => document.querySelectorAll(".skeleton").length === 0,
  );
  await page.waitForTimeout(150);
  await page.locator("#viewport").hover();
  for (let i = 0; i < 6; i++) {
    await page.mouse.wheel(0, 130);
    await page.waitForTimeout(20);
  }
  assert.equal(
    await page.locator(".skeleton").count(),
    0,
    "Wheel scrolling should use prefetched values",
  );
  await page.locator("#viewport").evaluate((v) => (v.scrollTop = 0));

  assert(
    (await page.locator(".data-row").count()) < 100,
    "Only the viewport should be mounted",
  );
  assert((await page.locator("#page-size").inputValue()) === "0");
  assert.equal(
    await page.evaluate(() => document.querySelectorAll(".value img").length),
    0,
    "CSV must render as text",
  );
  const settled = () =>
    page.waitForFunction(
      () =>
        document.querySelector("#table-busy").hidden &&
        document.querySelectorAll(".cell .value").length > 0,
    );
  async function search(value) {
    await page.locator("#search").fill(value);
    await page.locator("#search").press("Enter");
    await settled();
  }

  // Global search from page one must find a row far beyond the old 50k window.
  await page.locator("#page-size").selectOption("100");
  await search("NEEDLE_LAST");
  await page.waitForFunction(() =>
    document
      .querySelector("#view-summary")
      .textContent.startsWith("Найдено 1 из"),
  );
  assert.equal(
    await page.locator(".cell.first .value").first().textContent(),
    "последняя-запись",
  );
  assert.equal(
    (await page.locator(".row-number").first().textContent()).replace(
      /\D/g,
      "",
    ),
    String(size),
  );
  await page.screenshot({ path: resolve(artifacts, "global-search.png") });
  await search("");
  await page.locator("#next").click();
  await page.waitForFunction(() =>
    document.querySelector("#page-label").textContent.startsWith("2 /"),
  );
  await page.waitForFunction(
    () =>
      document.querySelector(".cell.first .value")?.textContent ===
      "процесс-000100",
  );
  await page.locator("#page-size").selectOption("0");

  // Chronological sort understands local and ISO dates together.
  await page
    .getByRole("button", { name: "Сортировать: Создан", exact: true })
    .click();
  await page.waitForFunction(
    () =>
      document.querySelector(".cell.first .value")?.textContent ===
      "последняя-запись",
  );
  await settled();
  await page
    .getByRole("button", { name: "Сортировать: Создан", exact: true })
    .click();
  await page.waitForFunction(
    () =>
      document.querySelector(".cell.first .value")?.textContent ===
      "процесс-000000",
  );
  await settled();
  await page
    .getByRole("button", { name: "Сортировать: PID", exact: true })
    .click();
  await page.waitForFunction(
    () =>
      document.querySelector(".cell.first .value")?.textContent ===
      "последняя-запись",
  );
  await settled();

  // Filters intersect the global search; first column cannot be hidden.
  await page.locator("#filters").click();
  await page
    .getByRole("button", { name: "+ Добавить условие", exact: true })
    .click();
  await page.getByLabel("Столбец фильтра", { exact: true }).selectOption("3");
  await page
    .getByLabel("Условие фильтра", { exact: true })
    .selectOption("equals");
  await page.getByLabel("Значение фильтра", { exact: true }).fill("Остановлен");
  await page.getByRole("button", { name: "Применить", exact: true }).click();
  await page.waitForFunction(
    (expected) =>
      document
        .querySelector("#view-summary")
        .textContent.replace(/\D/g, "")
        .startsWith(String(expected)),
    Math.ceil(size / 3),
  );
  await settled();
  await page.locator("#columns").click();
  assert(await page.locator(".columns-list input").first().isDisabled());
  await page.getByRole("button", { name: "Применить", exact: true }).click();
  await page.getByRole("button", { name: "Сбросить всё", exact: true }).click();
  await settled();

  // Native scroll goes to the final record without mounting 500k DOM rows.
  await page
    .locator("#viewport")
    .evaluate((v) => (v.scrollTop = v.scrollHeight));
  await page.waitForFunction(() =>
    [...document.querySelectorAll(".cell.first .value")].some(
      (v) => v.textContent === "последняя-запись",
    ),
  );
  assert((await page.locator(".data-row").count()) < 100);
  const pinnedBefore = await page.evaluate(
    () => document.querySelector(".cell.first").getBoundingClientRect().x,
  );
  await page.locator("#viewport").evaluate((v) => (v.scrollLeft = 600));
  await settled();
  const pinnedAfter = await page.evaluate(
    () => document.querySelector(".cell.first").getBoundingClientRect().x,
  );
  assert(
    Math.abs(pinnedBefore - pinnedAfter) < 1,
    "First column should stay pinned",
  );
  await page.locator("#viewport").evaluate((v) => {
    v.scrollTop = 0;
    v.scrollLeft = 0;
  });
  await settled();

  // Inspector loads the untruncated underlying cell and displays text safely.
  await page.locator(".cell.first").first().dblclick();
  await page.waitForFunction(
    () =>
      document.querySelector("#cell-value").textContent === "процесс-000000",
  );
  await page.locator("#close-inspector").click();
  await page.locator("#toolbar-settings").click();
  await page
    .getByLabel("Высота строк", { exact: false })
    .selectOption("compact");
  await page
    .getByLabel("Строк на странице", { exact: false })
    .last()
    .selectOption("500");
  await page.getByRole("button", { name: "Сохранить", exact: true }).click();
  assert.equal(await page.locator("#page-size").inputValue(), "500");
  assert.equal(
    await page.evaluate(
      () => JSON.parse(localStorage.getItem("timeline.settings")).compact,
    ),
    true,
  );
  await page.setViewportSize({ width: 960, height: 640 });
  await settled();
  await page.screenshot({ path: resolve(artifacts, "table-960.png") });
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
    false,
    "No page-wide overflow",
  );
  await page.locator("#toolbar-settings").click();
  await page.screenshot({ path: resolve(artifacts, "settings.png") });
  await page.locator("#dialog-close").click();
  await page.keyboard.press("Control+Equal");
  await page.keyboard.press("Control+Equal");
  await page.keyboard.press("Control+Equal");
  await zoomed(1.5);
  const widthAtZoom = await headerWidth(1);
  await resizeColumn(1, -40);
  assert(
    Math.abs((await headerWidth(1)) - (widthAtZoom - 40)) < 2,
    "Resizing uses the correct coordinates at 150% zoom",
  );
  await page.locator('.header-cell[data-column="1"] .resize-handle').dblclick();
  await page.screenshot({ path: resolve(artifacts, "table-zoom-150.png") });
  await page.keyboard.press("Control+Equal");
  await page.keyboard.press("Control+Equal");
  await zoomed(2);
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
    false,
    "200% zoom must not overflow the page",
  );
  await page.locator("#toolbar-settings").click();
  assert(
    await page
      .locator("#dialog-content")
      .evaluate((content) => content.clientHeight > 100),
    "Settings remain usable at 200% zoom",
  );
  await page.screenshot({ path: resolve(artifacts, "settings-zoom-200.png") });
  await page.locator("#dialog-close").click();
  await page.keyboard.press("Control+0");
  await zoomed(1);
  const wideFirstFile = resolve(artifacts, "wide-first-column.csv");
  await writeFile(
    wideFirstFile,
    `Название,Значение,Путь\n${"Название-".repeat(45)},Доступно,${"путь-".repeat(90)}\n`,
  );
  await page.evaluate(
    (path) => window.timelineReceive({ event: "open", path }),
    wideFirstFile,
  );
  await page.waitForFunction(() =>
    document
      .querySelector(".cell.first .value")
      ?.textContent.startsWith("Название-"),
  );
  await settled();
  await page.waitForFunction(() => {
    const value = document.querySelector('[aria-colindex="2"] .value');
    if (!value) return false;
    const rect = value.getBoundingClientRect(),
      port = document.querySelector("#viewport").getBoundingClientRect();
    return (
      value.textContent === "Доступно" &&
      rect.left >= port.left &&
      rect.right <= port.right
    );
  });
  const pinnedPositions = () =>
    page.evaluate(() => [
      document.querySelector(".cell.first").getBoundingClientRect().x,
      document.querySelector(".header-cell.first").getBoundingClientRect().x,
    ]);
  const firstPositions = await pinnedPositions();
  for (const offset of [300, 700, 10000, 0]) {
    await page
      .locator("#viewport")
      .evaluate((v, offset) => (v.scrollLeft = offset), offset);
    await page.waitForTimeout(100);
    const positions = await pinnedPositions();
    positions.forEach((position, i) =>
      assert(
        Math.abs(position - firstPositions[i]) < 1,
        "Even a very long first column must stay completely pinned",
      ),
    );
  }
  await resizeColumn(0, -160);
  assert(
    (await headerWidth(0)) < 300,
    "A pinned auto-sized name can also shrink",
  );
  await resizeColumn(0, 100);
  await page.locator('.header-cell[data-column="0"] .resize-handle').dblclick();
  // Optional pinning can be switched off directly from the existing pin icon.
  await page
    .getByRole("button", { name: "Открепить первый столбец", exact: true })
    .click();
  await page.waitForFunction(
    () => !document.querySelector(".cell.first").classList.contains("pinned"),
  );
  assert(
    (await headerWidth(0)) > 900,
    "An unpinned first column uses its full automatic width",
  );
  const wideUnpinnedOrigin = await firstColumnPositions();
  await page.locator("#viewport").evaluate((v) => (v.scrollLeft = 180));
  await page.waitForFunction(
    () => document.querySelector(".header-cell.first").style.left === "-180px",
  );
  (await firstColumnPositions()).forEach((position, i) =>
    assert(Math.abs(position - (wideUnpinnedOrigin[i] - 180)) < 1),
  );
  await page.locator("#viewport").evaluate((v) => {
    v.scrollLeft = document
      .querySelector(".cell.first")
      .getBoundingClientRect().width;
  });
  await page.locator('.cell[data-column="1"]').first().click();
  await page.keyboard.press("Home");
  await page.waitForFunction(
    () => document.querySelector("#viewport").scrollLeft === 0,
  );
  await page.locator("#toolbar-settings").click();
  assert.equal(
    await page
      .getByRole("checkbox", { name: "Закрепить первый столбец", exact: true })
      .isChecked(),
    false,
  );
  await page.locator("#dialog-close").click();
  await page.keyboard.press("Control+Equal");
  await page.keyboard.press("Control+Equal");
  await zoomed(1.25);
  await page.reload();
  await page.locator("#welcome-open").waitFor();
  await zoomed(1.25);
  await page.keyboard.press("Control+0");
  await zoomed(1);
  await page.evaluate(
    (path) => window.timelineReceive({ event: "open", path }),
    wideFirstFile,
  );
  await page.locator(".cell.first .value").first().waitFor();
  assert.equal(
    await page
      .locator(".cell.first")
      .first()
      .evaluate((cell) => getComputedStyle(cell).position),
    "absolute",
    "Unpinning persists after reloading and reopening a file",
  );
  await page.screenshot({ path: resolve(artifacts, "unpinned-table.png") });

  // Exercise identities, hidden columns, optional pinning and edge scrolling after reorder.
  const reorderFile = resolve(artifacts, "reorder.csv");
  const reorderHeaders = [
    "Название",
    "Дата",
    "Номер",
    "Статус",
    ...Array.from({ length: 28 }, (_, i) => `Поле ${i + 1}`),
  ];
  const reorderRows = [
    ["alpha", "03.04.2027 10:30:00", "10", "Скрыть"],
    ["beta", "31.12.2025 23:59:59", "2", "Показать"],
    ["gamma", "01.06.2026 12:00:00", "1", "Показать"],
  ];
  await writeFile(
    reorderFile,
    [
      reorderHeaders.join(","),
      ...reorderRows.map((row) =>
        [
          ...row,
          ...Array.from({ length: 28 }, (_, i) => `${row[0]}-${i}`),
        ].join(","),
      ),
    ].join("\n"),
  );
  await page.evaluate(
    (path) => window.timelineReceive({ event: "open", path }),
    reorderFile,
  );
  await page.waitForFunction(
    () =>
      document.querySelector('.cell[data-column="0"] .value')?.textContent ===
      "alpha",
  );
  await settled();
  await orderStartsWith([0, 1, 2, 3]);
  await page.locator("#columns").click();
  await page.getByRole("checkbox", { name: /^Статус/ }).uncheck();
  await page.getByRole("button", { name: "Применить", exact: true }).click();
  await orderStartsWith([0, 1, 2, 4]);
  await settled();
  await resizeColumn(2, 70);
  const resizedNumberWidth = await headerWidth(2);
  await dragColumn(2, 0);
  await orderStartsWith([2, 0, 1, 4]);
  assert.equal(
    await headerWidth(2),
    resizedNumberWidth,
    "Manual width follows the moved column",
  );
  await page
    .getByRole("button", { name: "Сортировать: Дата", exact: true })
    .click();
  await page.waitForFunction(
    () =>
      document.querySelector('.cell[data-column="0"] .value')?.textContent ===
      "beta",
  );
  await orderStartsWith([2, 0, 1, 4]);
  await page
    .getByRole("button", { name: "Настроить столбец Номер", exact: true })
    .click();
  await page
    .getByRole("button", { name: "+ Добавить условие", exact: true })
    .click();
  await page
    .getByLabel("Условие фильтра", { exact: true })
    .selectOption("equals");
  await page.getByLabel("Значение фильтра", { exact: true }).fill("2");
  await page.getByRole("button", { name: "Применить", exact: true }).click();
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".data-row").length === 1 &&
      document.querySelector('.cell[data-column="0"] .value')?.textContent ===
        "beta",
  );
  await orderStartsWith([2, 0, 1, 4]);
  await page.locator('.cell[data-column="2"]').first().dblclick();
  await page.waitForFunction(
    () => document.querySelector("#cell-value").textContent === "2",
  );
  await page.locator("#close-inspector").click();
  await page.getByRole("button", { name: "Сбросить всё", exact: true }).click();
  await page.waitForFunction(
    () => document.querySelectorAll(".data-row").length === 3,
  );
  await page
    .getByRole("button", { name: "Закрепить первый столбец", exact: true })
    .click();
  await dragColumn(0, 2);
  await orderStartsWith([0, 2, 1, 4]);
  assert.equal(
    await page.locator(".header-cell.pinned").getAttribute("data-column"),
    "0",
  );
  await page
    .getByRole("button", { name: "Открепить первый столбец", exact: true })
    .click();
  await dragColumn(1, 4, "after");
  await orderStartsWith([0, 2, 4, 1]);
  await page.locator("#columns").click();
  await page.getByRole("checkbox", { name: /^Статус/ }).check();
  await page.getByRole("button", { name: "Применить", exact: true }).click();
  await orderStartsWith([0, 2, 3, 4, 1]);
  await page.locator('.cell[data-column="2"]').first().click();
  await page.waitForFunction(() =>
    document
      .querySelector("#data-grid")
      .getAttribute("aria-activedescendant")
      ?.endsWith("-2"),
  );
  await page.keyboard.press("ArrowRight");
  await page.waitForFunction(() =>
    document
      .querySelector("#data-grid")
      .getAttribute("aria-activedescendant")
      ?.endsWith("-3"),
  );
  assert.match(
    await page.locator("#data-grid").getAttribute("aria-activedescendant"),
    /-3$/,
  );
  await page.keyboard.press("Home");
  await page.waitForFunction(() =>
    document
      .querySelector("#data-grid")
      .getAttribute("aria-activedescendant")
      ?.endsWith("-0"),
  );
  assert.match(
    await page.locator("#data-grid").getAttribute("aria-activedescendant"),
    /-0$/,
  );
  await page.keyboard.press("Control+Equal");
  await page.keyboard.press("Control+Equal");
  await page.keyboard.press("Control+Equal");
  await zoomed(1.5);
  await dragColumn(2, 0);
  await orderStartsWith([2, 0, 3]);
  const beforeCancel = await headerOrder();
  await dragColumn(0, 2, "before", true);
  assert.deepEqual(await headerOrder(), beforeCancel, "Escape cancels a drag");
  await page.keyboard.press("Control+0");
  await zoomed(1);
  await dragColumn(0, 2);
  await orderStartsWith([0, 2, 3]);
  const fromEdge = await page
    .locator('.header-cell[data-column="2"] .sort-button')
    .boundingBox();
  const edge = await page.locator("#table-header").boundingBox();
  await page.mouse.move(fromEdge.x + 35, fromEdge.y + fromEdge.height / 2);
  await page.mouse.down();
  await page.mouse.move(edge.x + edge.width - 18, edge.y + edge.height / 2, {
    steps: 12,
  });
  await page.waitForFunction(
    () => document.querySelector("#viewport").scrollLeft > 500,
  );
  await page.screenshot({ path: resolve(artifacts, "column-drag.png") });
  await page.mouse.up();
  await page.waitForTimeout(180);
  await page.locator("#columns").click();
  const finalColumnNames = await page
    .locator(".columns-list .check-row > span")
    .allTextContents();
  assert.equal(finalColumnNames.length, reorderHeaders.length);
  assert(
    finalColumnNames.indexOf("Номер") >= 5,
    "Holding at the edge can move beyond the initially visible columns",
  );
  assert.equal(new Set(finalColumnNames).size, reorderHeaders.length);
  await page.locator("#dialog-close").click();
  await page.locator("#viewport").evaluate((v) => (v.scrollLeft = 0));
  await page.screenshot({ path: resolve(artifacts, "reordered-table.png") });

  // Switch languages on a loaded table without translating its contents or losing state.
  const languageFile = resolve(artifacts, "language.csv");
  await writeFile(
    languageFile,
    "Текст,Столбцы,Дата\nНастройки,Загрузка…,03.04.2027\nEnglish,Keep data,31.12.2025\n",
  );
  await page.evaluate(
    (path) => window.timelineReceive({ event: "open", path }),
    languageFile,
  );
  await page.waitForFunction(
    () =>
      document.querySelector('.cell[data-column="0"] .value')?.textContent ===
      "Настройки",
  );
  await settled();
  await dragColumn(2, 0);
  await orderStartsWith([2, 0, 1]);
  await page.locator("#search").fill("Настройки");
  await page.locator("#search").press("Enter");
  await page.waitForFunction(
    () => document.querySelectorAll(".data-row").length === 1,
  );
  await page.locator('.cell[data-column="0"]').first().dblclick();
  await page.waitForFunction(
    () => document.querySelector("#cell-value").textContent === "Настройки",
  );
  await page.waitForFunction(() =>
    document
      .querySelector("#data-grid")
      .getAttribute("aria-activedescendant")
      ?.endsWith("-0"),
  );
  const selectionBeforeLanguage = await page
    .locator("#data-grid")
    .getAttribute("aria-activedescendant");
  await page.locator("#toolbar-settings").click();
  await page.getByLabel("Язык интерфейса", { exact: false }).selectOption("en");
  await page.getByRole("button", { name: "Сохранить", exact: true }).click();
  await page.waitForFunction(() => document.documentElement.lang === "en");
  assert.equal(await page.locator("#filters").innerText(), "Filters");
  assert.equal(await page.locator("#search").inputValue(), "Настройки");
  await orderStartsWith([2, 0, 1]);
  await page
    .getByRole("button", { name: "Sort: Текст", exact: true })
    .waitFor();
  await page.waitForFunction(
    () => document.querySelector("#cell-value").textContent === "Настройки",
  );
  assert.equal(
    await page.locator("#data-grid").getAttribute("aria-activedescendant"),
    selectionBeforeLanguage,
  );
  assert(
    (await page.locator("#view-summary").textContent()).startsWith(
      "Found 1 of 2",
    ),
  );
  await page.locator("#close-inspector").click();
  await page.locator("#filters").click();
  await page
    .getByRole("button", { name: "+ Add condition", exact: true })
    .click();
  await page.getByLabel("Filter column", { exact: true }).selectOption("1");
  await page
    .getByLabel("Filter condition", { exact: true })
    .selectOption("equals");
  await page.getByLabel("Filter value", { exact: true }).fill("Загрузка…");
  await page.getByRole("button", { name: "Apply", exact: true }).click();
  await settled();
  assert.equal(await page.locator("#filter-count").textContent(), "1");
  await page.locator("#toolbar-settings").click();
  await page
    .getByLabel("Interface language", { exact: false })
    .selectOption("ru");
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  assert.equal(
    await page.locator("html").getAttribute("lang"),
    "en",
    "Cancel must keep the existing language",
  );
  await page.reload();
  await page
    .getByRole("button", { name: "Choose file", exact: true })
    .waitFor();
  assert.equal(
    await page.locator("html").getAttribute("lang"),
    "en",
    "Saved language overrides the RU system region",
  );
  const generatedFile = resolve(artifacts, "generated-headers.csv");
  await writeFile(generatedFile, "Текст,,Column 2\nНастройки,Загрузка…,Keep\n");
  await page.evaluate(
    (path) => window.timelineReceive({ event: "open", path }),
    generatedFile,
  );
  await page
    .getByRole("button", { name: "Sort: Column 2 (2)", exact: true })
    .waitFor();
  await page
    .getByRole("button", { name: "Sort: Текст", exact: true })
    .waitFor();
  await page.locator("#toolbar-settings").click();
  await page
    .getByLabel("Interface language", { exact: false })
    .selectOption("auto");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await page
    .getByRole("button", { name: "Сортировать: Столбец 2", exact: true })
    .waitFor();
  await page.locator("#toolbar-settings").click();
  await page.getByLabel("Язык интерфейса", { exact: false }).selectOption("en");
  await page
    .getByLabel("Высота строк", { exact: false })
    .selectOption("normal");
  await page
    .getByLabel("Строк на странице", { exact: false })
    .last()
    .selectOption("0");
  await page.getByRole("button", { name: "Сохранить", exact: true }).click();
  await page.evaluate(
    (path) => window.timelineReceive({ event: "open", path }),
    resolve(artifacts, "missing-file.csv"),
  );
  await page.waitForFunction(
    () =>
      !document.querySelector("#notice").hidden &&
      document
        .querySelector("#notice-text")
        .textContent.includes("missing-file.csv"),
  );
  assert(!/[А-Яа-яЁё]/.test(await page.locator("#notice-text").textContent()));
  await page.locator("#notice-close").click();
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.evaluate(
    (path) => window.timelineReceive({ event: "open", path }),
    resolve(root, "samples/demo.csv"),
  );
  await page.waitForFunction(
    () =>
      document.querySelector(".cell.first .value")?.textContent === "System",
  );
  await settled();
  await page.screenshot({ path: resolve(artifacts, "interface-en.png") });
  await page.locator("#toolbar-settings").click();
  await page
    .getByLabel("Interface language", { exact: false })
    .selectOption("ru");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await page.waitForFunction(() => document.documentElement.lang === "ru");
  await page.screenshot({ path: resolve(artifacts, "interface-ru.png") });

  // Observe real bridge responses; replace only the OS clipboard destination.
  await page.evaluate(() => {
    window.__copied = null;
    window.__openedPath = null;
    const receive = window.timelineReceive;
    window.timelineReceive = (message) => {
      if (message.ok && message.data?.path)
        window.__openedPath = message.data.path;
      if (message.ok && message.data?.kind && message.data?.path)
        window.__openedKind = message.data.kind;
      receive(message);
    };
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text) => {
          window.__copied = text;
        },
      },
    });
  });
  async function openFixture(name, content) {
    const path = resolve(artifacts, name);
    await writeFile(path, content);
    await page.evaluate((path) => {
      window.__openedPath = null;
      window.timelineReceive({ event: "open", path });
    }, path);
    await page.waitForFunction(
      (path) =>
        window.__openedPath === path &&
        document.querySelector("#job-bar").hidden,
      path,
    );
    return path;
  }
  async function copied(expected, shortcut = "Control+KeyC") {
    await page.evaluate(() => {
      window.__copied = null;
    });
    if (shortcut) await page.keyboard.press(shortcut);
    else await page.locator("#copy-selection").click();
    await page.waitForFunction(() => window.__copied !== null);
    assert.equal(await page.evaluate(() => window.__copied), expected);
  }

  await openFixture(
    "selection.csv",
    "Имя,Число,Скрытый,Описание\nalpha,10,secret-a,first\nbeta,2,secret-b,second\ngamma,1,secret-c,third\n",
  );
  await settled();
  await dragColumn(1, 0);
  await orderStartsWith([1, 0, 2, 3]);
  await page.locator("#columns").click();
  await page.getByRole("checkbox", { name: /^Скрытый/ }).uncheck();
  await page.getByRole("button", { name: "Применить", exact: true }).click();
  await orderStartsWith([1, 0, 3]);
  await page
    .getByRole("button", { name: "Сортировать: Число", exact: true })
    .click();
  await page.waitForFunction(
    () =>
      document.querySelector('.cell[data-column="0"] .value')?.textContent ===
      "gamma",
  );
  await page.locator('.cell[data-index="0"][data-column="1"]').click();
  await page
    .locator('.cell[data-index="2"][data-column="3"]')
    .click({ modifiers: ["Shift"] });
  await page.waitForFunction(
    () => document.querySelectorAll('.cell[aria-selected="true"]').length === 9,
  );
  await copied("1\tgamma\tthird\r\n2\tbeta\tsecond\r\n10\talpha\tfirst");
  await page.screenshot({ path: resolve(artifacts, "range-selection.png") });
  // A pointer rectangle has the same visible order as Shift-click.
  const dragStart = await page
    .locator('.cell[data-index="0"][data-column="1"]')
    .boundingBox();
  const dragEnd = await page
    .locator('.cell[data-index="1"][data-column="3"]')
    .boundingBox();
  await page.mouse.move(
    dragStart.x + dragStart.width / 2,
    dragStart.y + dragStart.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    dragEnd.x + dragEnd.width / 2,
    dragEnd.y + dragEnd.height / 2,
    { steps: 8 },
  );
  await page.waitForFunction(
    () => document.querySelectorAll('.cell[aria-selected="true"]').length === 6,
  );
  await page.mouse.up();
  await copied("1\tgamma\tthird\r\n2\tbeta\tsecond", null);
  // Hiding a selected endpoint must not silently copy a different column.
  await page.locator("#columns").click();
  await page.getByRole("checkbox", { name: /^Описание/ }).uncheck();
  await page.getByRole("button", { name: "Применить", exact: true }).click();
  await page.waitForFunction(
    () => document.querySelector("#copy-selection").disabled,
  );
  assert.equal(await page.locator('.cell[aria-selected="true"]').count(), 0);

  const manyRows = Array.from({ length: 1501 }, (_, i) => `row-${i},${i}`);
  await openFixture(
    "selection-offscreen.csv",
    "Имя,Значение\n" + manyRows.join("\n"),
  );
  await settled();
  await page.locator('.cell[data-index="0"][data-column="0"]').click();
  await page.keyboard.press("Control+Shift+End");
  await page.keyboard.press("Shift+ArrowRight");
  await page.waitForFunction(() =>
    document.querySelector('.cell[data-index="1500"][data-column="1"]'),
  );
  assert((await page.locator(".data-row").count()) < 100);
  await copied(manyRows.map((line) => line.replace(",", "\t")).join("\r\n"));
  // A drag on a short final page cannot select invisible rows from another page.
  await page.locator("#page-size").selectOption("100");
  await page.locator('.cell[data-index="0"][data-column="0"]').click();
  await page.keyboard.press("Control+End");
  await page.waitForFunction(() =>
    document.querySelector('.cell[data-index="1500"]'),
  );
  const last = await page
    .locator('.cell[data-index="1500"][data-column="0"]')
    .boundingBox();
  const portRect = await page.locator("#viewport").boundingBox();
  await page.mouse.move(last.x + 100, last.y + last.height / 2);
  await page.mouse.down();
  await page.mouse.move(last.x + 110, portRect.y + portRect.height - 2, {
    steps: 8,
  });
  await page.mouse.up();
  await copied("row-1500");
  await page.locator("#page-size").selectOption("0");

  const plainText =
    "Заметки по исследованию\n\nСтрока с отступом остаётся частью текста.\n" +
    "Длинный абзац сохраняет пробелы и читается с переносом строк. ".repeat(
      35,
    ) +
    "\nПоследняя запись: ✓ завершено.\n";
  await openFixture("notes.txt", plainText);
  await page.locator(".cm-content").waitFor();
  assert(await page.locator("#table-pane").isHidden());
  assert(await page.locator("#document-mode").isHidden());
  assert(await page.locator("#document-wrap").isChecked());
  await page.waitForFunction(() =>
    document.querySelector(".cm-content").classList.contains("cm-lineWrapping"),
  );
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+Home");
  await page.keyboard.press("Shift+End");
  await copied("Заметки по исследованию");
  // Reconfiguring wrapping must preserve the original text and the selection.
  await page.locator("#document-wrap").uncheck();
  assert.equal(
    await page.evaluate(() => window.timelineViewer.selectedText()),
    "Заметки по исследованию",
  );
  await page.evaluate(() => {
    window.__copied = null;
  });
  await page.locator("#document-copy").click();
  await page.waitForFunction(() => window.__copied !== null);
  assert.equal(
    await page.evaluate(() => window.__copied),
    "Заметки по исследованию",
  );
  await page.locator("#document-wrap").focus();
  await page.keyboard.press("Control+f");
  await page.locator(".cm-search input[name=search]").fill("завершено");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Escape");
  assert.equal(
    await page.evaluate(() => window.timelineViewer.selectedText()),
    "завершено",
  );
  await openFixture("notes.log", plainText);
  await page.locator(".cm-content").waitFor();
  assert(!(await page.locator("#document-wrap").isChecked()));
  assert.equal(
    await page.evaluate(
      () => JSON.parse(localStorage.getItem("timeline.settings")).documentWrap,
    ),
    false,
  );
  await page.locator("#document-wrap").check();
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+Home");
  await page.evaluate(() => {
    window.__copied = null;
  });
  await page.locator("#document-copy").click();
  await page.waitForFunction(() => window.__copied !== null);
  assert.equal(await page.evaluate(() => window.__copied), plainText);
  await page.screenshot({ path: resolve(artifacts, "text-reader.png") });
  // Tabular TXT still opens directly as a table.
  await openFixture(
    "tabular.txt",
    "Name\tPID\tDescription\nfirst\t12\tStarted\nsecond\t42\tStopped\n",
  );
  await settled();
  assert(await page.locator("#document-pane").isHidden());
  assert.equal(await page.locator(".header-cell").count(), 3);

  const binaryUrl = Buffer.from(
    (
      await readFile(resolve(root, "tests/fixtures/binary-url.hex"), "utf8")
    ).replace(/\s/g, ""),
    "hex",
  );
  await openFixture("url-payload.dat", binaryUrl);
  await page.waitForFunction(() => window.__openedKind === "binary");
  await page.waitForFunction(() =>
    [...document.querySelectorAll('.cell[data-column="3"] .value')].some(
      (cell) => cell.textContent.includes("http://example.test/"),
    ),
  );
  assert.match(
    await page.locator("#notice-text").textContent(),
    /Извлечённые строки/,
  );
  await page.screenshot({ path: resolve(artifacts, "binary-readable.png") });
  await page.locator("#binary-toggle").click();
  await page.waitForFunction(
    () =>
      window.__openedKind === "hex" &&
      document.querySelector("#job-bar").hidden,
  );
  await page
    .getByRole("button", { name: "Сортировать: Hex", exact: true })
    .waitFor();
  await page.locator("#binary-toggle").click();
  await page.waitForFunction(
    () =>
      window.__openedKind === "binary" &&
      document.querySelector("#job-bar").hidden,
  );
  await openFixture("unrecognized.dat", Buffer.alloc(128, 0xff));
  await page.waitForFunction(() =>
    [...document.querySelectorAll('.cell[data-column="3"] .value')].some(
      (cell) => cell.textContent.includes("Читаемый текст не обнаружен"),
    ),
  );

  const hexCellText = "http://example.test/проверка";
  const hexCell = Buffer.from(hexCellText + "\0", "utf16le")
    .toString("hex")
    .match(/../g)
    .join(" ")
    .toUpperCase();
  await openFixture("hex-cell.csv", "Name,Value\nURL," + hexCell + "\n");
  await settled();
  await page.locator('.cell[data-index="0"][data-column="1"]').dblclick();
  await page.waitForFunction(
    (text) => document.querySelector("#cell-value").textContent === text,
    hexCellText,
  );
  assert(await page.locator("#cell-mode").isVisible());
  await page.evaluate(() => {
    window.__copied = null;
  });
  await page.locator("#copy-cell").click();
  await page.waitForFunction(() => window.__copied !== null);
  assert.equal(await page.evaluate(() => window.__copied), hexCellText);
  await page.locator("#cell-mode").selectOption("raw");
  assert.equal(await page.locator("#cell-value").textContent(), hexCell);
  await page.evaluate(() => {
    window.__copied = null;
  });
  await page.locator("#copy-cell").click();
  await page.waitForFunction(() => window.__copied !== null);
  assert.equal(await page.evaluate(() => window.__copied), hexCell);
  await page.locator("#cell-mode").selectOption("decoded");
  await page.screenshot({ path: resolve(artifacts, "hex-cell-readable.png") });

  const preciseJson =
    '{"wide":18446744073709551615,"decimal":0.12345678901234567890123456789,"nested":{"items":[true,null,"text"]}}';
  await openFixture("precise.json", preciseJson);
  await page.locator(".json-leaf").first().waitFor();
  assert.match(
    await page.locator("#document-content").innerText(),
    /18446744073709551615/,
  );
  assert.match(
    await page.locator("#document-content").innerText(),
    /0\.12345678901234567890123456789/,
  );
  await page.getByText('"nested" {1}', { exact: true }).click();
  await page.getByText('"items" [3]', { exact: true }).click();
  await page.getByText('[2]: "text"', { exact: true }).waitFor();
  await page.screenshot({ path: resolve(artifacts, "json-tree.png") });
  await page.locator("#document-mode").selectOption("source");
  await page.locator(".cm-line").first().waitFor();
  assert((await page.locator(".cm-line").count()) > 3);
  await page.locator("#document-raw").check();
  await page.waitForFunction(
    () => document.querySelectorAll(".cm-line").length === 1,
  );
  assert.equal(await page.locator(".cm-line").innerText(), preciseJson);
  assert(await page.locator("#document-mode").isDisabled());

  await openFixture(
    "records.jsonl",
    '{"id":9007199254740993,"message":"one"}\n{"id":9007199254740995,"message":"two"}\n',
  );
  await page.locator("#document-toggle").click();
  await page.locator("#document-mode").selectOption("tree");
  await page.getByText("[1] {2}", { exact: true }).click();
  await page.getByText('"id": 9007199254740995', { exact: true }).waitFor();

  const markdown =
    '# Evidence\n\nText **bold** and `code`.\n\n| Name | Value |\n| --- | --- |\n| Run | 42 |\n\n<script>window.__unsafeMarkdown=true</script>\n<img src="https://example.test/track" onerror="window.__unsafeMarkdown=true">\n';
  await openFixture("evidence.md", markdown);
  await page.locator(".markdown-body h1").waitFor();
  assert.equal(await page.locator(".markdown-body h1").innerText(), "Evidence");
  assert.equal(await page.locator(".markdown-body table").count(), 1);
  assert.equal(
    await page.locator(".markdown-body script, .markdown-body img").count(),
    0,
  );
  assert.equal(await page.evaluate(() => window.__unsafeMarkdown), undefined);
  await page.screenshot({ path: resolve(artifacts, "markdown-preview.png") });
  await page.locator("#document-raw").check();
  await page.locator(".cm-line").first().waitFor();
  assert.equal(
    await page.locator(".cm-line").first().innerText(),
    "# Evidence",
  );
  assert.equal(await page.locator(".markdown-body").count(), 0);

  const script =
    'function evidence() {\n\tconst path = "C:\\\\Windows";\n  return path;\n}\n';
  await openFixture("source.js", script);
  await page.locator(".cm-line").first().waitFor();
  assert.equal(
    (await page.locator(".cm-gutterElement").filter({ hasText: "1" }).count()) >
      0,
    true,
  );
  assert.equal(
    await page.locator('.cm-content[contenteditable="false"]').count(),
    1,
  );
  await page.locator(".cm-content").click();
  await page.keyboard.press("Control+f");
  await page.locator(".cm-search input[name=search]").waitFor();
  await page.locator(".cm-search input[name=search]").fill("evidence");
  await page.keyboard.press("Escape");
  await page.screenshot({ path: resolve(artifacts, "source-viewer.png") });
  await page.setViewportSize({ width: 620, height: 600 });
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
    false,
  );
  await page.setViewportSize({ width: 1280, height: 800 });

  // Source chunks retain absolute line numbers and the final original content.
  const longLine = "// " + "x".repeat(2040) + "\n";
  await openFixture(
    "notes-chunks.txt",
    longLine.repeat(4300) + "const finalEvidence = 9007199254740993n;\n",
  );
  await page.locator(".cm-line").first().waitFor();
  await page.locator("#document-next").click();
  await page.waitForFunction(() =>
    document
      .querySelector("#document-range")
      .textContent.startsWith("Фрагмент 2"),
  );
  const firstLineNumber = await page
    .locator(".cm-lineNumbers .cm-gutterElement")
    .nth(1)
    .innerText();
  assert(
    Number(firstLineNumber) > 4000,
    "Source chunks use absolute line numbers",
  );
  assert(await page.locator("#document-next").isDisabled());
  await page.locator("#document-content").click();
  await page.keyboard.press("Control+End");
  await page.waitForFunction(() =>
    document.querySelector(".cm-content").textContent.includes("finalEvidence"),
  );
  await page.screenshot({ path: resolve(artifacts, "source-chunk.png") });
  await page.locator("#document-prev").click();
  await page.waitForFunction(() =>
    document
      .querySelector("#document-range")
      .textContent.startsWith("Фрагмент 1"),
  );
  assert(await page.locator("#document-prev").isDisabled());
  assert.equal(
    await page.locator(".cm-lineNumbers .cm-gutterElement").nth(1).innerText(),
    "1",
  );

  assert.deepEqual(errors, []);
  console.log(
    "PASS: native search, date/number sorting, filters, pagination, 500257 virtual rows, column order/pinning/widths, keyboard/wheel zoom, persistence, cell inspector, settings, small-window layout, drag/Shift/offscreen range copying, text/log reader and table detection, persistent word wrap, text copying, readable DAT and Hex views, decoded/raw hex cell copying, precise JSON/JSONL trees, Markdown raw/preview, read-only source search and paged line numbers.",
  );
} catch (error) {
  if (page) {
    await page
      .screenshot({ path: resolve(artifacts, "failure.png") })
      .catch(() => {});
    console.error(
      await page
        .locator("body")
        .innerText()
        .catch(() => ""),
    );
  }
  console.error(errors);
  throw error;
} finally {
  if (browser) await browser.close().catch(() => {});
  app.kill();
}
