// OCS AnswererWrapper data handler. Reads only the current question's page.
// Selectors and normalization follow OCS 4.15.3's Chaoxing chapter/work adapters.
function collectPageQuestions(env) {
  const limit = 200;
  const clean = value => String(value || "").replace(/\s+/g, " ").trim();
  const optionKey = value => String(value || "").split("\n").map(clean).filter(Boolean).join("\n");
  const titleKey = clean(env.title);
  if (!titleKey) return [];
  const typeMap = {0:"single",1:"multiple",2:"completion",3:"judgement",4:"completion",5:"completion",6:"completion",7:"completion",8:"completion",9:"completion",10:"completion"};
  const defaults = "单选题(必考)\n填空题(必考)\n多选题(必考)\n(单选题)\n(多选题)\n(判断题)\n(填空题)\n【单选题】\n【多选题】\n【填空题】\n【判断题】\n【單選题】\n【多選题】\n【判斷题】\n【Single Choice】\n【Multiple Choice】\n【single choice】\n【multiple choice】\n【True or False】";
  const words = (globalThis.OCS?.CommonProject?.scripts?.settings?.cfg?.redundanceWordsText || defaults).split("\n");
  const removeWords = text => words.reduce((value, word) => value.replace(word.trim(), ""), text);
  const text = element => {
    if (!element) return "";
    const copy = element.cloneNode(true);
    for (const img of copy.querySelectorAll("img")) {
      if (!Array.from(img.parentElement.querySelectorAll("span")).some(span => span.style.fontSize === "0px" && span.textContent.includes(img.src))) {
        const span = copy.ownerDocument.createElement("span");
        span.textContent = img.src;
        img.after(span);
      }
    }
    return copy.innerText || copy.textContent || "";
  };
  function parse(root, chapter) {
    const input = root.querySelector('input[id^="answertype"], input[name^="type"]');
    const type = typeMap[Number(input?.value)];
    if (!input || !type) return null;
    let title;
    if (chapter) {
      const titles = [".Zy_TItle .clearfix", ".firstUlList", ".secondUlList"].map(s => root.querySelector(s)).filter(Boolean);
      title = removeWords(titles.map(text).join(",")).trim()
        .replace(/^\d+[。、.]/, "").replace(/（\d+\.\d+分）/, "")
        .replace(/\(..题, \d+?分\)/, "").replace(/\(..题, \d+\.\d+分\)/, "")
        .replace(/[[(【（](..题|名词解释|完形填空|阅读理解)[\])】）]/, "").trim();
    } else {
      const titles = Array.from(root.querySelectorAll(".mark_name, .line_wid_half.fl, .line_wid_half.fr"))
        .filter(el => el.textContent.trim());
      if (!titles.length) return null;
      title = titles.map((el, index) => {
        const copy = el.cloneNode(true);
        if (index === 0) { copy.firstChild?.remove(); copy.firstChild?.remove(); }
        return text(copy);
      }).join("\n");
      title = removeWords(title.replace(/\n/g, titles.length > 1 ? "\n" : " ").replace(/ +/g, " ").trim());
    }
    if (!title) return null;
    const selector = chapter ? "ul li .after,ul li textarea,ul textarea,ul li label:not(.before)" : ".answerBg .answer_p, .textDIV, .eidtDiv";
    const options = type === "completion" ? "" : Array.from(root.querySelectorAll(selector)).map(text).join("\n");
    return {title, options, type};
  }
  const seen = new Set();
  function scan(win, depth) {
    if (depth > 6) return [];
    let doc;
    try { doc = win.document; } catch { return []; } // Respect cross-origin frame boundaries.
    if (!doc || seen.has(doc)) return [];
    seen.add(doc);
    for (const [selector, chapter] of [[".TiMu", true], [".questionLi", false]]) {
      const questions = Array.from(doc.querySelectorAll(selector)).map(root => parse(root, chapter)).filter(Boolean);
      const matches = questions.map((q, index) => ({q, index})).filter(({q}) =>
        clean(q.title) === titleKey && (!env.type || q.type === env.type) &&
        (!env.options || optionKey(q.options) === optionKey(env.options)));
      if (matches.length === 1) {
        const start = Math.floor(matches[0].index / limit) * limit;
        return questions.slice(start, start + limit);
      }
    }
    for (const frame of doc.querySelectorAll("iframe, frame")) {
      let found;
      try { found = scan(frame.contentWindow, depth + 1); } catch { continue; }
      if (found.length) return found;
    }
    return [];
  }
  let start = window;
  try { if (window.top.document) start = window.top; } catch { /* Scan this accessible frame. */ }
  return scan(start, 0);
}
