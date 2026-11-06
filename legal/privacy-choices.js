// Shared by both Atlas sites. Generated copies must match this file.
(() => {
  const key = "atlas.analytics-choice.v1";
  const excluded = location.pathname.startsWith("/mailing-list/") || location.pathname.startsWith("/checkout/");
  const signal = () => navigator.globalPrivacyControl === true || navigator.doNotTrack === "1";
  const read = () => { try { return localStorage.getItem(key); } catch { return "essential"; } };
  const allowed = () => !excluded && !signal() && read() === "allow";
  // Cookieless, anonymous visit measurement (OpenPanel): on by default, off with GPC/DNT or on excluded pages.
  const measure = () => !excluded && !signal();
  let panel;
  function save(value) {
    try { localStorage.setItem(key, value); } catch { /* remain essential-only */ }
    panel?.remove(); panel = undefined;
    window.dispatchEvent(new Event("atlas:privacy-choice"));
    if (value === "essential") location.reload();
  }
  function show() {
    if (excluded || panel) return;
    panel = document.createElement("section");
    panel.setAttribute("aria-label", "Website privacy choices");
    panel.setAttribute("role", "region");
    Object.assign(panel.style, {position:"fixed",bottom:"16px",left:"16px",right:"16px",margin:"0 auto",maxWidth:"700px",zIndex:"2147483000",background:"#17191c",color:"#fff",border:"1px solid #60656c",borderRadius:"12px",padding:"20px",boxShadow:"0 8px 40px #0006",font:"14px/1.6 system-ui,sans-serif"});
    const text = document.createElement("p");
    text.textContent = signal() ? "Your browser privacy signal is enabled. All optional analytics stay off." : "We count visits anonymously with OpenPanel, without cookies. PostHog analytics and session replays only run if you allow them. Essential security, checkout and email services work either way.";
    text.style.margin = "0 0 12px";
    panel.append(text);
    const actions = document.createElement("div");
    Object.assign(actions.style, {display:"flex",flexWrap:"wrap",gap:"12px",alignItems:"center"});
    for (const [label, value] of [["Anonymous only", "essential"], ["Allow all analytics", "allow"]]) {
      if (value === "allow" && signal()) continue;
      const button = document.createElement("button");
      button.type = "button"; button.textContent = label;
      Object.assign(button.style, {background:"#fff",color:"#111",border:"1px solid #fff",borderRadius:"6px",padding:"9px 14px",font:"inherit",cursor:"pointer"});
      button.addEventListener("click", () => save(value)); actions.append(button);
    }
    const link = document.createElement("a"); link.href = "/privacy-policy"; link.textContent = "Read privacy policy"; link.style.color = "#fff"; link.style.textDecoration = "underline"; actions.append(link);
    panel.append(actions); document.body.append(panel);
  }
  window.AtlasPrivacy = {allowed, measure, show};
  window.addEventListener("atlas:privacy-settings", show);
  window.addEventListener("storage", event => { if (event.key === key && event.newValue !== "allow") location.reload(); });
  document.addEventListener("click", event => { if (event.target.closest?.("[data-privacy-choices]")) show(); });
  function ready() { if (!read() && !excluded) show(); window.dispatchEvent(new Event("atlas:privacy-choice")); }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", ready, {once:true}); else ready();
})();
