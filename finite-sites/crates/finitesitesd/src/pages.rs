//! Inline HTML for platform-rendered pages (placeholders, login, errors).
//! Site names are DNS labels and emails are validated before they reach
//! these templates, so interpolation cannot inject markup.

const STYLE: &str = "
  :root {
    --bg-window: #212121;
    --bg-card: #171615;
    --bg-elevated: #252525;
    --bg-hover: rgba(255,255,255,0.055);
    --text-primary: #ececec;
    --text-secondary: #a6a19d;
    --text-tertiary: #706d69;
    --border: rgba(255,255,255,0.085);
    --border-strong: rgba(255,255,255,0.16);
    --accent-blue: #8ab4ff;
    --button-text: #f6f8ff;
    --shadow: 0 24px 70px rgba(0,0,0,0.34);
    color-scheme: dark;
  }
  @media (prefers-color-scheme: light) {
    :root {
      --bg-window: #f7f6f3;
      --bg-card: #fffdfa;
      --bg-elevated: #f1efeb;
      --bg-hover: rgba(20,20,20,0.055);
      --text-primary: #171717;
      --text-secondary: #66615b;
      --text-tertiary: #918a82;
      --border: rgba(20,20,20,0.105);
      --border-strong: rgba(20,20,20,0.18);
      --accent-blue: #315fbd;
      --button-text: #f6f8ff;
      --shadow: 0 24px 70px rgba(20,20,20,0.12);
      color-scheme: light;
    }
  }
  * { box-sizing: border-box; }
  body {
    min-height: 100vh;
    min-height: 100dvh;
    margin: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: var(--bg-window);
    color: var(--text-primary);
    font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", system-ui, sans-serif;
    font-size: 15px;
    -webkit-font-smoothing: antialiased;
  }
  main {
    width: min(100%, 440px);
    padding: 28px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg-card);
    box-shadow: var(--shadow);
  }
  .eyebrow {
    margin: 0 0 10px;
    color: var(--text-tertiary);
    font-size: 12px;
    font-weight: 600;
    line-height: 1.3;
  }
  h1 {
    margin: 0;
    color: var(--text-primary);
    font-size: 24px;
    line-height: 1.15;
    font-weight: 600;
    letter-spacing: 0;
  }
  p {
    margin: 12px 0 0;
    color: var(--text-secondary);
    line-height: 1.45;
  }
  form {
    margin-top: 22px;
    display: grid;
    gap: 10px;
  }
  input[type=email] {
    width: 100%;
    min-height: 44px;
    padding: 0 13px;
    border-radius: 8px;
    border: 1px solid var(--border-strong);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font: inherit;
    outline: none;
  }
  input[type=email]::placeholder { color: var(--text-tertiary); }
  input[type=email]:focus {
    border-color: color-mix(in srgb, var(--accent-blue), var(--border-strong) 25%);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-blue), transparent 78%);
  }
  button,
  .agent-link {
    display: inline-flex;
    min-height: 44px;
    align-items: center;
    justify-content: center;
    border-radius: 999px;
    padding: 0 18px;
    font: inherit;
    font-weight: 600;
    text-decoration: none;
  }
  button {
    border: 0;
    background: var(--text-primary);
    color: var(--bg-window);
    cursor: pointer;
  }
  button:hover { opacity: 0.92; }
  .agent-cta {
    margin-top: 18px;
    padding-top: 18px;
    border-top: 1px solid var(--border);
  }
  .agent-cta p { margin: 0 0 10px; font-size: 13px; }
  .agent-link {
    width: 100%;
    border: 1px solid var(--border-strong);
    background: transparent;
    color: var(--accent-blue);
  }
  .agent-link:hover { background: var(--bg-hover); }
  .brand { margin-top: 22px; font-size: 12px; color: var(--text-tertiary); }
  @media (min-width: 520px) {
    form { grid-template-columns: 1fr auto; }
  }
";

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title}</title><style>{STYLE}</style></head>\
         <body><main>{body}<p class=\"brand\">finite sites</p></main></body></html>"
    )
}

pub fn unknown_site() -> String {
    page(
        "No such site",
        "<h1>No site lives here</h1>\
         <p>This address is not claimed. It could be yours.</p>",
    )
}

pub fn placeholder(name: &str) -> String {
    page(
        name,
        &format!(
            "<h1>{name} is claimed</h1>\
             <p>Nothing has been published here yet. Check back soon.</p>"
        ),
    )
}

pub fn login(name: &str) -> String {
    page(
        &format!("Sign in to {name}"),
        &format!(
            "<p class=\"eyebrow\">{name}</p>\
             <h1>This site is private</h1>\
             <p>If {name} has been shared with you, enter your email and \
             we&rsquo;ll send you a sign-in link.</p>\
             <form method=\"post\" action=\"/_finite/request-link\">\
               <input type=\"email\" name=\"email\" placeholder=\"you@example.com\" required>\
               <button type=\"submit\">Send link</button>\
             </form>\
             <div class=\"agent-cta\">\
               <p>Working with an agent?</p>\
               <a class=\"agent-link\" href=\"/llms.txt\">Open llms.txt</a>\
             </div>"
        ),
    )
}

pub fn link_sent() -> String {
    page(
        "Check your email",
        "<h1>Check your email</h1>\
         <p>A verification link is on its way. After you verify the address, \
         we&rsquo;ll tell you whether the site has been shared with you. \
         The link can be reused and expires in 15 minutes.</p>",
    )
}

pub fn link_invalid() -> String {
    page(
        "Link expired",
        "<h1>That link didn&rsquo;t work</h1>\
         <p>Sign-in links expire after 15 minutes. \
         Request a fresh one from the site&rsquo;s sign-in page.</p>",
    )
}

pub fn not_shared(email: &str) -> String {
    page(
        "Request access",
        &format!(
            "<h1>You don&rsquo;t have access yet</h1>\
             <p>You verified {email}, but this site has not been shared with \
             that address.</p>\
             <form method=\"post\" action=\"/_finite/request-access\">\
               <button type=\"submit\">Request access</button>\
             </form>"
        ),
    )
}

pub fn access_requested() -> String {
    page(
        "Access requested",
        "<h1>Request sent</h1>\
         <p>We emailed the site owner. This page will open after they approve \
         access; refresh it when you hear back.</p>",
    )
}

pub fn approve_access_confirmation(site_name: &str, email: &str, token: &str) -> String {
    page(
        "Approve access",
        &format!(
            "<p class=\"eyebrow\">{site_name}</p>\
             <h1>Approve access?</h1>\
             <p>{email} verified this mailbox and requested access.</p>\
             <form method=\"post\" action=\"/_finite/approve-access\">\
               <input type=\"hidden\" name=\"token\" value=\"{token}\">\
               <button type=\"submit\">Share this site</button>\
             </form>"
        ),
    )
}

pub fn access_approved(site_name: &str, email: &str) -> String {
    page(
        "Access approved",
        &format!(
            "<p class=\"eyebrow\">{site_name}</p>\
             <h1>Access approved</h1>\
             <p>{email} can now sign in and view this site.</p>\
             <p><a class=\"agent-link\" href=\"/\">Open site</a></p>"
        ),
    )
}

pub fn not_found() -> String {
    page(
        "Not found",
        "<h1>404</h1><p>This page does not exist on this site.</p>",
    )
}
