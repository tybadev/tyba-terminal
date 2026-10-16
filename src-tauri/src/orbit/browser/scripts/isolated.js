// Mundo isolado "tyba-orbit" (Fase 5, `fase5-navegador`): o que o agente
// lê da página. Roda num `WKContentWorld` próprio — a página não vê nada
// daqui (nem `__orbit`, nem as refs, nem o buffer), e daqui só se vê o DOM.
//
// Entra duas vezes, sempre idempotente: como user script no início do
// documento (para o buffer do console começar cedo) e no corpo de cada ação
// (página que carregou antes do user script). As refs `e<N>` ficam num `Map`
// deste mundo, nunca em atributo do DOM: a página não lê nem forja. Valem
// até o próximo snapshot ou a próxima página.
//
// Os argumentos chegam como argumento (`callAsyncJavaScript`), nunca
// interpolados. O retorno é dado: o core redige, corta e marca.
(() => {
  if (globalThis.__orbit) return;

  const MAX_NODES = 1500;
  const MAX_WALK = 6000;
  const MAX_NAME = 200;
  const MAX_TEXT = 300;
  const MAX_VALUE = 200;
  const MAX_CONSOLE = 200;
  const MAX_ENTRY = 500;

  // ── console ────────────────────────────────────────────────────────────
  const consoleBuf = [];
  let errors = 0;
  const describe = (v) => {
    if (typeof v === "string") return v;
    if (v instanceof Error) return v.stack || `${v.name}: ${v.message}`;
    try {
      return JSON.stringify(v);
    } catch {
      return String(v);
    }
  };
  const push = (level, text) => {
    if (level !== "warn") errors++;
    consoleBuf.push({ level, text: String(text).slice(0, MAX_ENTRY) });
    if (consoleBuf.length > MAX_CONSOLE) consoleBuf.shift();
  };
  window.addEventListener("__tyba_console", (e) => {
    const d = typeof e.detail === "string" ? e.detail : "";
    const i = d.indexOf(":");
    push(d.slice(0, i) === "warn" ? "warn" : "error", d.slice(i + 1));
  });
  window.addEventListener("error", (e) => {
    push("uncaught", e.message || describe(e.error) || "erro");
  });
  window.addEventListener("unhandledrejection", (e) => push("rejection", describe(e.reason)));

  // ── papel e nome ───────────────────────────────────────────────────────
  const squash = (s) =>
    String(s || "")
      .replace(/\s+/g, " ")
      .trim();
  const INPUT_ROLES = {
    button: "button",
    submit: "button",
    reset: "button",
    image: "button",
    checkbox: "checkbox",
    radio: "radio",
    range: "slider",
    number: "spinbutton",
    search: "searchbox",
  };
  const TAG_ROLES = {
    BUTTON: "button",
    SUMMARY: "button",
    TEXTAREA: "textbox",
    H1: "heading",
    H2: "heading",
    H3: "heading",
    H4: "heading",
    H5: "heading",
    H6: "heading",
    NAV: "navigation",
    MAIN: "main",
    FORM: "form",
    UL: "list",
    OL: "list",
    LI: "listitem",
    TABLE: "table",
    TR: "row",
    TD: "cell",
    TH: "columnheader",
    DIALOG: "dialog",
    OPTION: "option",
  };
  const INTERACTIVE = new Set([
    "link",
    "button",
    "textbox",
    "searchbox",
    "checkbox",
    "radio",
    "combobox",
    "listbox",
    "option",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "treeitem",
  ]);
  // Papéis cujo nome já é o texto de dentro: não desce.
  const LEAF = new Set([...INTERACTIVE, "heading", "img"]);
  const SKIP = new Set([
    "SCRIPT",
    "STYLE",
    "NOSCRIPT",
    "TEMPLATE",
    "HEAD",
    "META",
    "LINK",
    "IFRAME",
    "OBJECT",
    "EMBED",
  ]);
  const TEXT_INPUTS = new Set(["text", "email", "password", "search", "tel", "url", "number", ""]);

  const hidden = (el) => {
    if (el.hidden || el.getAttribute("aria-hidden") === "true") return true;
    const s = getComputedStyle(el);
    return s.display === "none" || s.visibility === "hidden" || s.visibility === "collapse";
  };

  const roleOf = (el) => {
    const explicit = squash(el.getAttribute("role")).split(" ")[0];
    if (explicit) return explicit;
    const tag = el.tagName;
    if (tag === "A" || tag === "AREA") return el.hasAttribute("href") ? "link" : "";
    if (tag === "INPUT") {
      const t = (el.getAttribute("type") || "text").toLowerCase();
      if (t === "hidden") return "";
      return INPUT_ROLES[t] || "textbox";
    }
    if (tag === "SELECT") return el.multiple || el.size > 1 ? "listbox" : "combobox";
    if (tag === "IMG") return el.getAttribute("alt") === "" ? "" : "img";
    if (el.isContentEditable && !(el.parentElement && el.parentElement.isContentEditable))
      return "textbox";
    return TAG_ROLES[tag] || "";
  };

  const textOf = (el) => squash(el.innerText !== undefined ? el.innerText : el.textContent);

  const nameOf = (el, role) => {
    const aria = squash(el.getAttribute("aria-label"));
    if (aria) return aria;
    const by = el.getAttribute("aria-labelledby");
    if (by) {
      const t = squash(
        by
          .split(/\s+/)
          .map((id) => {
            const r = document.getElementById(id);
            return r ? textOf(r) : "";
          })
          .join(" "),
      );
      if (t) return t;
    }
    const tag = el.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") {
      const type = (el.getAttribute("type") || "").toLowerCase();
      if (tag === "INPUT" && ["button", "submit", "reset"].includes(type))
        return squash(el.value) || (type === "submit" ? "Submit" : "");
      if (tag === "INPUT" && type === "image") return squash(el.getAttribute("alt"));
      if (el.labels && el.labels.length) {
        const t = squash([...el.labels].map(textOf).join(" "));
        if (t) return t;
      }
      return squash(
        el.getAttribute("placeholder") || el.getAttribute("title") || el.getAttribute("name"),
      );
    }
    if (tag === "IMG") return squash(el.getAttribute("alt") || el.getAttribute("title"));
    if (LEAF.has(role)) return textOf(el) || squash(el.getAttribute("title"));
    return squash(el.getAttribute("title"));
  };

  const isPassword = (el) =>
    el.tagName === "INPUT" && (el.getAttribute("type") || "").toLowerCase() === "password";

  const hrefOf = (el) => {
    const raw = el.getAttribute("href") || "";
    try {
      const u = new URL(raw, location.href);
      if (u.origin === location.origin) return u.pathname + u.search + u.hash;
      return u.href;
    } catch {
      return raw;
    }
  };

  // ── refs ───────────────────────────────────────────────────────────────
  let refs = new Map();
  let next = 1;
  const fail = (code, detail) => {
    const e = new Error(code);
    e.orbit = code;
    e.detail = detail || "";
    return e;
  };
  const element = (ref) => {
    const el = refs.get(String(ref));
    if (!el || !el.isConnected) throw fail("staleRef");
    return el;
  };
  const target = (el) => {
    const role = roleOf(el) || el.tagName.toLowerCase();
    return { role, name: nameOf(el, role).slice(0, MAX_NAME) };
  };

  // ── snapshot ───────────────────────────────────────────────────────────
  const snapshot = () => {
    refs = new Map();
    next = 1;
    const nodes = [];
    let total = 0;
    let capped = false;
    const add = (n) => {
      total++;
      if (nodes.length < MAX_NODES) nodes.push(n);
    };
    const walk = (parent, depth) => {
      for (const child of parent.childNodes) {
        if (total >= MAX_WALK) {
          capped = true;
          return;
        }
        if (child.nodeType === 3) {
          const t = squash(child.data);
          if (t) add({ d: depth, role: "text", name: t.slice(0, MAX_TEXT) });
          continue;
        }
        if (child.nodeType !== 1 || SKIP.has(child.tagName) || hidden(child)) continue;
        const role = roleOf(child);
        if (!role) {
          walk(child, depth);
          continue;
        }
        const n = { d: depth, role, name: nameOf(child, role).slice(0, MAX_NAME) };
        if (role === "heading") {
          const lvl = Number(child.getAttribute("aria-level")) || Number(child.tagName.slice(1));
          if (lvl) n.level = lvl;
        }
        if (role === "link") n.href = hrefOf(child);
        if (child.disabled || child.getAttribute("aria-disabled") === "true") n.disabled = true;
        if (role === "checkbox" || role === "radio" || role === "switch") {
          const aria = child.getAttribute("aria-checked");
          n.checked = aria ? aria === "true" : !!child.checked;
        }
        const expanded = child.getAttribute("aria-expanded");
        if (expanded) n.expanded = expanded === "true";
        if (isPassword(child)) n.secret = true;
        else if (child.tagName === "INPUT" || child.tagName === "TEXTAREA") {
          if (role === "textbox" || role === "searchbox" || role === "spinbutton")
            n.value = String(child.value || "").slice(0, MAX_VALUE);
        } else if (child.tagName === "SELECT") {
          n.value = squash([...child.selectedOptions].map((o) => o.text).join(", ")).slice(
            0,
            MAX_VALUE,
          );
        }
        if (INTERACTIVE.has(role)) {
          const r = `e${next++}`;
          refs.set(r, child);
          n.ref = r;
        }
        add(n);
        if (!LEAF.has(role)) walk(child, depth + 1);
      }
    };
    walk(document.body || document.documentElement, 0);
    return { nodes, total, capped };
  };

  // ── agir ───────────────────────────────────────────────────────────────
  // Eventos sintéticos (`isTrusted = false`): página que exige evento de
  // verdade não reage (limite aceito na v1).
  const usable = (el) => {
    if (hidden(el)) throw fail("hidden");
    if (el.disabled || el.getAttribute("aria-disabled") === "true") throw fail("disabled");
  };

  const click = (a) => {
    const el = element(a.ref);
    usable(el);
    el.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) throw fail("hidden");
    const x = r.left + r.width / 2;
    const y = r.top + r.height / 2;
    if (x < 0 || y < 0 || x > innerWidth || y > innerHeight) throw fail("offscreen");
    // Não clica através de um overlay: o ponto central tem de cair nele (ou
    // no rótulo dele, o caso do checkbox escondido atrás do label).
    const hit = document.elementFromPoint(x, y);
    const label = hit && hit.closest ? hit.closest("label") : null;
    if (!hit || !(hit === el || el.contains(hit) || (label && label.control === el))) {
      if (!hit) throw fail("offscreen");
      const t = target(hit);
      throw fail("covered", `${t.role} "${t.name.slice(0, 80)}"`);
    }
    const base = {
      bubbles: true,
      cancelable: true,
      composed: true,
      clientX: x,
      clientY: y,
      button: 0,
      view: window,
    };
    const pointer = { ...base, pointerId: 1, pointerType: "mouse", isPrimary: true };
    el.dispatchEvent(new PointerEvent("pointerdown", { ...pointer, buttons: 1 }));
    el.dispatchEvent(new MouseEvent("mousedown", { ...base, buttons: 1, detail: 1 }));
    if (typeof el.focus === "function") el.focus({ preventScroll: true });
    el.dispatchEvent(new PointerEvent("pointerup", { ...pointer, buttons: 0 }));
    el.dispatchEvent(new MouseEvent("mouseup", { ...base, buttons: 0, detail: 1 }));
    el.dispatchEvent(new MouseEvent("click", { ...base, buttons: 0, detail: 1 }));
    return target(el);
  };

  const type = (a) => {
    const el = element(a.ref);
    usable(el);
    const text = String(a.text || "");
    const tag = el.tagName;
    el.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
    if (typeof el.focus === "function") el.focus({ preventScroll: true });
    if (tag === "INPUT" || tag === "TEXTAREA") {
      const kind = (el.getAttribute("type") || "").toLowerCase();
      if (tag === "INPUT" && !TEXT_INPUTS.has(kind)) throw fail("notText");
      if (el.readOnly) throw fail("readOnly");
      // O setter nativo (o React escuta o `input` depois dele).
      const proto = tag === "INPUT" ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
      Object.getOwnPropertyDescriptor(proto, "value").set.call(el, text);
      el.dispatchEvent(
        new InputEvent("input", {
          bubbles: true,
          composed: true,
          inputType: "insertText",
          data: text,
        }),
      );
      el.dispatchEvent(new Event("change", { bubbles: true }));
    } else if (el.isContentEditable) {
      const sel = getSelection();
      const range = document.createRange();
      range.selectNodeContents(el);
      sel.removeAllRanges();
      sel.addRange(range);
      document.execCommand("insertText", false, text);
    } else {
      throw fail("notText");
    }
    if (a.submit) {
      const form = el.form || (el.closest ? el.closest("form") : null);
      if (form && typeof form.requestSubmit === "function") form.requestSubmit();
      else if (form) form.submit();
      else
        for (const t of ["keydown", "keypress", "keyup"])
          el.dispatchEvent(
            new KeyboardEvent(t, {
              key: "Enter",
              code: "Enter",
              keyCode: 13,
              which: 13,
              bubbles: true,
              cancelable: true,
            }),
          );
    }
    return { ...target(el), chars: [...text].length };
  };

  const ACTIONS = {
    snapshot,
    click,
    type,
    // Só a url e o título (o envelope do `run` traz): depois de assentar.
    where: () => ({}),
    console: () => ({ entries: consoleBuf.slice() }),
  };

  globalThis.__orbit = {
    async run(action, args) {
      const page = { url: location.href, title: document.title };
      const fn = ACTIONS[action];
      if (!fn) return { error: "unknownAction", ...page, errors };
      try {
        const ok = await fn(args || {});
        return { ok, ...page, errors };
      } catch (e) {
        if (e && e.orbit) return { error: e.orbit, detail: e.detail, ...page, errors };
        return { error: "script", detail: String((e && e.message) || e).slice(0, 200), ...page, errors };
      }
    },
  };
})();
