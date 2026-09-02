//! Inline HTML for the gate's small page set. Inputs interpolated into
//! these templates are validated (`parse_output_origin`,
//! `parse_return_to`, bounded dev email) before they arrive here.

const STYLE: &str = "
  :root { color-scheme: dark light; }
  * { box-sizing: border-box; }
  body {
    min-height: 100vh; min-height: 100dvh; margin: 0;
    display: flex; align-items: center; justify-content: center;
    padding: 24px;
    background: #212121; color: #ececec;
    font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", system-ui, sans-serif;
    font-size: 15px; -webkit-font-smoothing: antialiased;
  }
  @media (prefers-color-scheme: light) {
    body { background: #f7f6f3; color: #171717; }
    main { background: #fffdfa; border-color: rgba(20,20,20,0.105); }
    .dev-banner { background: #3b2f00; }
  }
  main {
    width: min(100%, 440px); padding: 28px;
    border: 1px solid rgba(255,255,255,0.085); border-radius: 8px;
    background: #171615; box-shadow: 0 24px 70px rgba(0,0,0,0.34);
  }
  .eyebrow { margin: 0 0 10px; color: #706d69; font-size: 12px; font-weight: 600; }
  h1 { margin: 0; font-size: 24px; line-height: 1.15; font-weight: 600; }
  p { margin: 12px 0 0; color: #a6a19d; line-height: 1.45; }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 13px; word-break: break-all;
  }
  .dev-banner {
    margin: -28px -28px 20px; padding: 10px 16px; border-radius: 8px 8px 0 0;
    background: #5c4a00; color: #ffe9a3; font-size: 12px; font-weight: 700;
    letter-spacing: 0.02em; text-align: center;
  }
  form { margin-top: 22px; display: grid; gap: 10px; }
  button {
    display: inline-flex; min-height: 44px; align-items: center;
    justify-content: center; border-radius: 999px; padding: 0 18px;
    border: 0; background: #ececec; color: #212121;
    font: inherit; font-weight: 600; cursor: pointer;
  }
  button:hover { opacity: 0.92; }
  .brand { margin-top: 22px; font-size: 12px; color: #706d69; }
";

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title}</title><style>{STYLE}</style></head>\
         <body><main>{body}<p class=\"brand\">finite auth gate</p></main></body></html>"
    )
}

fn hidden(name: &str, value: &str) -> String {
    format!("<input type=\"hidden\" name=\"{name}\" value=\"{value}\">")
}

/// Dev-mode confirmation. Loudly labeled: this is NOT production
/// authentication, it mints a vouch for one fixed dev identity.
pub fn dev_confirm(dev_email: &str, audience: &str, return_to: &str) -> String {
    page(
        "Dev sign-in",
        &format!(
            "<div class=\"dev-banner\">DEV MODE — NOT PRODUCTION AUTHENTICATION</div>\
             <p class=\"eyebrow\">{audience}</p>\
             <h1>Continue as {dev_email}?</h1>\
             <p>WorkOS is not configured, so this local gate mints a sign-in \
             vouch for the fixed dev identity \
             <span class=\"mono\">{dev_email}</span> after one confirmation.</p>\
             <form method=\"post\" action=\"/dev/confirm\">\
               {}{}\
               <button type=\"submit\">Continue as {dev_email}</button>\
             </form>",
            hidden("output", audience),
            hidden("return_to", return_to),
        ),
    )
}

pub fn error(message: &str) -> String {
    page(
        "Sign-in problem",
        &format!(
            "<h1>Sign-in problem</h1>\
             <p>{message}</p>"
        ),
    )
}
