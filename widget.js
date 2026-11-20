/* lucanian.app fleet feedback widget — dependency-free vanilla JS.
 * No frameworks, no cookies, no fingerprinting, no tracking.
 * Config via data-* attributes on the script tag:
 *   data-product  (required)  product key, e.g. "gittrend"
 *   data-position (optional)  bottom-right | bottom-left | top-right | top-left
 * Integration:
 *   <script src="https://feedback.lucanian.app/widget.js" data-product="gittrend" defer></script>
 */
(function () {
  "use strict";
  var s = document.currentScript;
  if (!s || !s.dataset.product) return;
  var product = s.dataset.product;
  var pos = s.dataset.position || "bottom-right";
  var api = new URL(s.src).origin;
  var corner = { top: pos.indexOf("top") === 0 ? "16px" : "auto", bottom: pos.indexOf("top") === 0 ? "auto" : "16px", right: pos.indexOf("left") === -1 ? "16px" : "auto", left: pos.indexOf("left") === -1 ? "auto" : "16px" };
  function el(tag, css, text) { var e = document.createElement(tag); e.style.cssText = css; if (text != null) e.textContent = text; return e; }
  var btn = el("button", "position:fixed;" + (corner.top === "auto" ? "bottom:16px;" : "top:16px;") + (corner.left === "auto" ? "right:16px;" : "left:16px;") + "z-index:99999;background:#111827;color:#fff;border:0;border-radius:999px;padding:10px 18px;font:14px/1.2 system-ui,sans-serif;cursor:pointer;box-shadow:0 4px 14px rgba(0,0,0,.25)", "Feedback");
  var panel = el("div", "display:none;position:fixed;" + (corner.top === "auto" ? "bottom:64px;" : "top:64px;") + (corner.left === "auto" ? "right:16px;" : "left:16px;") + "z-index:99999;width:300px;background:#fff;color:#111;border-radius:12px;padding:16px;font:14px/1.4 system-ui,sans-serif;box-shadow:0 8px 30px rgba(0,0,0,.3)");
  var cats = el("div", "display:flex;gap:6px;margin-bottom:8px");
  var active = "useful";
  ["useful", "bug", "feature"].forEach(function (c) {
    var b = el("button", "flex:1;border:1px solid #d1d5db;background:#f9fafb;border-radius:8px;padding:6px 0;cursor:pointer;font:inherit", c);
    if (c === "useful") b.style.cssText += ";background:#111827;color:#fff;border-color:#111827";
    b.onclick = function () { active = c; Array.prototype.forEach.call(cats.children, function (x) { x.style.cssText = "flex:1;border:1px solid #d1d5db;background:#f9fafb;border-radius:8px;padding:6px 0;cursor:pointer;font:inherit"; }); b.style.cssText += ";background:#111827;color:#fff;border-color:#111827"; };
    cats.appendChild(b);
  });
  var ta = el("textarea", "width:100%;box-sizing:border-box;height:80px;border:1px solid #d1d5db;border-radius:8px;padding:8px;font:inherit;resize:vertical");
  ta.placeholder = "What happened? What would help?";
  var em = el("input", "width:100%;box-sizing:border-box;margin-top:8px;border:1px solid #d1d5db;border-radius:8px;padding:8px;font:inherit");
  em.type = "email";
  em.placeholder = "Email (optional) — only used to reply about this";
  // Honeypot: hidden from humans; bots that fill it are silently dropped server-side.
  var hp = el("input", "position:absolute;left:-9999px;top:-9999px;opacity:0;height:0;width:0");
  hp.type = "text"; hp.name = "website"; hp.tabIndex = -1; hp.autocomplete = "off";
  hp.setAttribute("aria-hidden", "true");
  var send = el("button", "margin-top:10px;width:100%;border:0;border-radius:8px;padding:9px 0;background:#111827;color:#fff;font:inherit;cursor:pointer", "Send feedback");
  var note = el("div", "margin-top:8px;font-size:12px;color:#6b7280;min-height:1em");
  var open = false;
  function toggle() { open = !open; panel.style.display = open ? "block" : "none"; if (open) note.textContent = ""; }
  btn.onclick = toggle;
  send.onclick = function () {
    note.textContent = "Sending…";
    fetch(api + "/api/feedback", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ product: product, page: location.pathname, category: active, message: ta.value, email: em.value || null, website: hp.value })
    }).then(function (r) {
      if (r.status === 429) { note.textContent = "Too many submissions — please try again in a few minutes."; return null; }
      if (!r.ok) { note.textContent = "Something went wrong (" + r.status + "). Please retry."; return null; }
      return r.json();
    }).then(function (j) {
      if (!j) return;
      var url = api + j.status_url;
      panel.textContent = "";
      panel.appendChild(el("div", "font-weight:600;margin-bottom:6px", "Thanks — feedback received!"));
      var a = el("a", "color:#2563eb;word-break:break-all", url);
      a.href = url;
      panel.appendChild(el("div", "font-size:12px;color:#6b7280;margin-bottom:4px", "Track its status anytime (save this link):"));
      panel.appendChild(a);
      var copy = el("button", "margin-top:10px;width:100%;border:1px solid #d1d5db;border-radius:8px;padding:8px 0;background:#f9fafb;font:inherit;cursor:pointer", "Copy status link");
      copy.onclick = function () {
        (navigator.clipboard ? navigator.clipboard.writeText(url) : Promise.reject()).then(function () { copy.textContent = "Copied!"; }, function () { copy.textContent = "Select & copy manually"; });
      };
      panel.appendChild(copy);
      ta.value = "";
    }).catch(function () { note.textContent = "Network error — please retry."; });
  };
  panel.appendChild(cats); panel.appendChild(ta); panel.appendChild(em); panel.appendChild(hp); panel.appendChild(send); panel.appendChild(note);
  if (document.body) { document.body.appendChild(btn); document.body.appendChild(panel); } else { document.addEventListener("DOMContentLoaded", function () { document.body.appendChild(btn); document.body.appendChild(panel); }); }
})();
