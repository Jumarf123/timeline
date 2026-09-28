import { Compartment, EditorState } from "@codemirror/state";
import {
  EditorView,
  lineNumbers,
  highlightActiveLineGutter,
  drawSelection,
  highlightSpecialChars,
} from "@codemirror/view";
import { defaultKeymap } from "@codemirror/commands";
import { keymap } from "@codemirror/view";
import {
  searchKeymap,
  highlightSelectionMatches,
  search,
  openSearchPanel,
} from "@codemirror/search";
import {
  foldGutter,
  syntaxHighlighting,
  defaultHighlightStyle,
  bracketMatching,
  foldKeymap,
} from "@codemirror/language";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { xml } from "@codemirror/lang-xml";
import { python } from "@codemirror/lang-python";
import { StreamLanguage } from "@codemirror/language";
import { powerShell as powershell } from "@codemirror/legacy-modes/mode/powershell";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { yaml } from "@codemirror/legacy-modes/mode/yaml";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { parse, stringify, isLosslessNumber } from "lossless-json";

let editor;
const wrapping = new Compartment();
const node = (tag, text, className) => {
  const n = document.createElement(tag);
  if (text !== undefined) n.textContent = text;
  if (className) n.className = className;
  return n;
};
function tree(parent, value, name = "$", depth = 0) {
  if (value === null || typeof value !== "object" || isLosslessNumber(value)) {
    parent.append(
      node(
        "div",
        `${name}: ${isLosslessNumber(value) ? value.value : JSON.stringify(value)}`,
        "json-leaf",
      ),
    );
    return;
  }
  const entries = Object.entries(value),
    details = node("details", undefined, "json-branch");
  details.append(
    node(
      "summary",
      `${name} ${Array.isArray(value) ? `[${entries.length}]` : `{${entries.length}}`}`,
    ),
  );
  let built = false,
    offset = 0;
  const children = node("div", undefined, "json-children");
  const more = node(
    "button",
    window.timelineI18n.t("Показать ещё"),
    "text-button",
  );
  const append = () => {
    more.remove();
    for (const [k, v] of entries.slice(offset, offset + 200))
      tree(
        children,
        v,
        Array.isArray(value) ? `[${k}]` : JSON.stringify(k),
        depth + 1,
      );
    offset += 200;
    if (offset < entries.length) children.append(more);
  };
  more.onclick = append;
  details.ontoggle = () => {
    if (details.open && !built) {
      built = true;
      append();
    }
  };
  details.append(children);
  parent.append(details);
  if (depth === 0) details.open = true;
}
function render(
  parent,
  text,
  kind,
  mode = "source",
  firstLine = 1,
  wrap = true,
) {
  editor?.destroy();
  editor = null;
  parent.replaceChildren();
  // The built-in search panel commits on keyup/change. Also commit input from
  // context-menu paste, IME and other input methods before the next Enter key.
  parent.oninput = (event) => {
    if (event.target.matches('.cm-search input[name="search"]'))
      event.target.dispatchEvent(new Event("change", { bubbles: true }));
  };
  let notice = "";
  if (mode === "tree") {
    try {
      const value =
        kind === "jsonl"
          ? text
              .split(/\r?\n/)
              .filter((line) => line.trim())
              .map((line) => parse(line))
          : parse(text);
      tree(parent, value);
      return { notice };
    } catch {
      notice = window.timelineI18n.t(
        "Дерево недоступно для этого фрагмента JSON. Показан исходный текст без потери данных.",
      );
    }
  }
  if (kind === "markdown" && mode === "preview") {
    const article = node("article", undefined, "markdown-body");
    // No remote images, executable HTML, navigation or local file links in evidence.
    article.innerHTML = DOMPurify.sanitize(marked.parse(text), {
      USE_PROFILES: { html: true },
      FORBID_TAGS: ["img", "style", "input", "iframe", "form"],
      FORBID_ATTR: ["href", "src", "style"],
    });
    parent.append(article);
    return { notice };
  }
  const languages = {
    javascript,
    json,
    jsonl: json,
    markdown,
    xml,
    python,
    powershell: () => StreamLanguage.define(powershell),
    shell: () => StreamLanguage.define(shell),
    yaml: () => StreamLanguage.define(yaml),
  };
  const extensions = [
    EditorState.readOnly.of(true),
    EditorView.editable.of(false),
    EditorView.contentAttributes.of({
      tabindex: "0",
      "aria-label": window.timelineI18n.t("Текст документа"),
    }),
    EditorState.phrases.of({
      Find: window.timelineI18n.t("Найти"),
      next: window.timelineI18n.t("Далее"),
      previous: window.timelineI18n.t("Назад"),
      all: window.timelineI18n.t("Все совпадения"),
      "match case": window.timelineI18n.t("Учитывать регистр"),
      regexp: "Regex",
      "by word": window.timelineI18n.t("Слово целиком"),
      close: window.timelineI18n.t("Закрыть"),
      "Go to line": window.timelineI18n.t("Перейти к строке"),
      go: window.timelineI18n.t("Перейти"),
    }),
    wrapping.of(wrap ? EditorView.lineWrapping : []),
    lineNumbers({ formatNumber: (n) => String(n + firstLine - 1) }),
    highlightActiveLineGutter(),
    drawSelection(),
    highlightSpecialChars(),
    search({ top: true }),
    keymap.of([...searchKeymap, ...foldKeymap, ...defaultKeymap]),
    highlightSelectionMatches(),
    foldGutter(),
    bracketMatching(),
    EditorView.theme({
      "&": { height: "100%", fontSize: "13px" },
      ".cm-scroller": {
        overflow: "auto",
        fontFamily: "Consolas, monospace",
        lineHeight: "1.65",
      },
      ".cm-content": { padding: "12px 0" },
      ".cm-gutters": { backgroundColor: "#f7f8fa", color: "#87929e" },
    }),
  ];
  if (mode !== "raw") {
    extensions.push(syntaxHighlighting(defaultHighlightStyle));
    if (languages[kind]) extensions.push(languages[kind]());
  }
  editor = new EditorView({
    parent,
    state: EditorState.create({ doc: text, extensions }),
  });
  return { notice };
}
window.timelineViewer = {
  pretty: (text) => {
    try {
      return stringify(parse(text), null, 2);
    } catch {
      return text;
    }
  },
  render,
  tree,
  hasEditor: () => Boolean(editor),
  search: () => editor && openSearchPanel(editor),
  selectedText: () =>
    editor?.state.selection.ranges
      .filter((range) => !range.empty)
      .map((range) => editor.state.sliceDoc(range.from, range.to))
      .join("\n") || "",
  setWrap: (wrap) => {
    editor?.dispatch({
      effects: wrapping.reconfigure(wrap ? EditorView.lineWrapping : []),
    });
  },
  destroy: () => {
    editor?.destroy();
    editor = null;
  },
};
