import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../web/i18n.js", import.meta.url), "utf8");
function create(region, preference = "auto") {
  const context = {
    window: { timelineRegion: region },
    document: { documentElement: {} },
    localStorage: { getItem: () => JSON.stringify({ language: preference }) },
    Intl,
  };
  vm.runInNewContext(source, context);
  return context.window.timelineI18n;
}

test("region defaults and saved language overrides", () => {
  for (const region of ["RU", "BY", "UA", "ru"])
    assert.equal(create(region).language, "ru");
  for (const region of ["US", "GB", "DE", "KZ", "", "001"])
    assert.equal(create(region).language, "en");
  assert.equal(create("RU", "en").language, "en");
  assert.equal(create("US", "ru").language, "ru");
  assert.equal(create("US", "invalid").language, "en");
});

test("language changes translate messages and preserve interpolation values", () => {
  const locale = create("RU");
  locale.setLanguage("en");
  assert.equal(
    locale.t("Сортировать: {name}", { name: "Текст" }),
    "Sort: Текст",
  );
  assert.equal(locale.number(21159937), "21,159,937");
  locale.setLanguage("auto");
  assert.equal(locale.t("Настройки"), "Настройки");
  assert.equal(locale.number(21159937).replace(/\s/g, ""), "21159937");
});

test("every app-owned string has an English translation with matching placeholders", () => {
  const { english } = create("US");
  const app = readFileSync(new URL("../web/app.js", import.meta.url), "utf8");
  const quoted = [...app.matchAll(/"(?:\\.|[^"\\])*"/g)].map((m) =>
    JSON.parse(m[0]),
  );
  const missing = quoted.filter(
    (text) =>
      /[А-Яа-яЁё]/.test(text) &&
      text !== "Русский" &&
      !Object.hasOwn(english, text),
  );
  assert.deepEqual([...new Set(missing)], []);
  const html = readFileSync(
    new URL("../web/index.html", import.meta.url),
    "utf8",
  );
  const staticText = [...html.matchAll(/>([^<>]+)</g)].map((m) =>
    m[1].trim().replace(/\s+/g, " "),
  );
  for (const text of staticText.filter((s) => /[А-Яа-яЁё]/.test(s)))
    assert(Object.hasOwn(english, text), `Missing static translation: ${text}`);
  for (const [russian, translated] of Object.entries(english)) {
    const tokens = (s) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
    assert.deepEqual(tokens(translated), tokens(russian), russian);
    assert(!/[А-Яа-яЁё]/.test(translated), russian);
  }
});
