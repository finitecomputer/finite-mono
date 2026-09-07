//! Outbound mail for Finite Sites. Two implementations behind one trait:
//!
//! - `DevMailer`: writes each magic-link email to a file under `DATA/outbox/`
//!   and logs the link. Selected with `--mailer dev`. Local development only;
//!   omitting `--mailer` is an error, not an implicit DevMailer.
//! - `HttpMailer`: sends through the shared `finite-mail` Resend transport.
//!   Selected with `--mailer resend`; the API key comes from the
//!   RESEND_API_KEY environment variable so secrets stay in the service env
//!   file, never in argv.
//!
//! Message text lives here (service-owned); delivery lives in `finite-mail`.

use std::path::PathBuf;

use finite_mail::{MailTransport, ResendMailer, TextEmail};
use finitesites_proto::dto::ProjectSiteSummary;

pub use finite_mail::MailError as MailerError;
pub use finite_mail::RESEND_API_KEY_ENV_VAR;

/// Which delivery mode `--mailer` selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailerKind {
    Dev,
    Resend,
}

impl MailerKind {
    pub fn parse(value: &str) -> Option<MailerKind> {
        match value {
            "dev" => Some(MailerKind::Dev),
            "resend" => Some(MailerKind::Resend),
            _ => None,
        }
    }
}

pub trait Mailer: Send + Sync {
    fn send_login_link(&self, email: &str, site_name: &str, url: &str) -> Result<(), MailerError>;
    fn send_email_login_token(&self, email: &str, token: &str) -> Result<(), MailerError>;
    fn send_viewer_invite(&self, invite: &ViewerInvite<'_>) -> Result<(), MailerError>;
    fn send_project_collaborator_invite(
        &self,
        invite: &ProjectCollaboratorInvite<'_>,
    ) -> Result<(), MailerError>;
    fn send_site_access_request(
        &self,
        request: &SiteAccessRequestEmail<'_>,
    ) -> Result<(), MailerError>;
    fn send_first_publication(
        &self,
        email: &str,
        site_name: &str,
        site_url: &str,
    ) -> Result<(), MailerError>;
}

pub struct ViewerInvite<'a> {
    pub email: &'a str,
    pub site_name: &'a str,
    pub site_url: &'a str,
    pub login_url: &'a str,
}

pub struct ProjectCollaboratorInvite<'a> {
    pub email: &'a str,
    pub project_slug: &'a str,
    pub role: &'a str,
    pub api_url: &'a str,
    pub git_remote_url: &'a str,
    pub email_login_token: &'a str,
    pub site: Option<&'a ProjectSiteSummary>,
}

pub struct SiteAccessRequestEmail<'a> {
    pub owner_email: &'a str,
    pub requester_email: &'a str,
    pub site_name: &'a str,
    pub site_url: &'a str,
    pub approval_url: &'a str,
}

/// Message text is shared by every mailer so dev output matches what real
/// recipients see.
fn login_link_subject(site_name: &str) -> String {
    format!("Your link to {site_name}")
}

fn login_link_text(site_name: &str, url: &str) -> String {
    format!(
        "Open this link to view {site_name}:\n\n{url}\n\n\
         The link can be reused and expires in 15 minutes. If you did not \
         request it, you can ignore this email.\n"
    )
}

fn email_login_subject() -> &'static str {
    "Your Finite Sites email login"
}

fn email_login_text(email: &str, token: &str) -> String {
    format!(
        "Run this command to verify {email} for Finite Sites publishing:\n\n\
         fsite auth redeem {email} {token}\n\n\
         The token works once and expires in 15 minutes. If you did not \
         request it, you can ignore this email.\n"
    )
}

fn viewer_invite_subject(site_name: &str) -> String {
    format!("You've been invited to view {site_name}")
}

fn viewer_invite_text(invite: &ViewerInvite<'_>) -> String {
    format!(
        "{site_name} has been shared with you.\n\n\
         To view it, open this sign-in link:\n\n{login_url}\n\n\
         After signing in, you can view the site here:\n\n{site_url}\n\n\
         The sign-in link can be reused and expires in 15 minutes. If it expires, \
         open the site URL and request a fresh link for {email}.\n\n\
         For your agent\n\n\
         If you use an agent with this site, ask it to read this email. The canonical \
         site URL is:\n\n{site_url}\n\n\
         Agent instructions:\n\n{llms_url}\n",
        site_name = invite.site_name,
        login_url = invite.login_url,
        site_url = invite.site_url,
        llms_url = site_url_with_path(invite.site_url, "/llms.txt"),
        email = invite.email,
    )
}

fn project_collaborator_invite_subject(project_slug: &str) -> String {
    format!("You've been invited to collaborate on {project_slug}")
}

fn project_collaborator_invite_text(invite: &ProjectCollaboratorInvite<'_>) -> String {
    let api_prefix = api_prefix(invite.api_url);
    let mut text = format!(
        "You have been invited to collaborate on {project_slug} as {role}.\n\n\
         If you use an agent, ask it to read the \"For your agent\" section below.\n\n\
         For your agent\n\n\
         Inspect the current edit workflow with:\n\n\
         {api_prefix}fsite describe workflow edit-shared-project --output json\n\n\
         Preferred native auth path. This links {email} to the local npub, so future email grants can resolve without email-only git auth:\n\n\
         {api_prefix}fsite auth register --output json\n\
         {api_prefix}fsite auth redeem {email} {token} --link-native --output json\n\n\
         Then mint a scoped git credential and clone the project:\n\n\
         {api_prefix}fsite auth git {project_slug} --store --output json\n\
         git clone {git_remote_url}\n\n\
         Email-only fallback. If you do not want to link this email to a native npub, run:\n\n\
         {api_prefix}fsite auth redeem {email} {token}\n\n\
         Then mint a scoped email git credential and clone the project:\n\n\
         {api_prefix}fsite auth git {project_slug} --email {email} --store --output json\n\
         git clone {git_remote_url}\n\n\
         Edit the repository, commit your changes, and push the deploy branch.\n\
         The email token works once and expires in 15 minutes. If it expires, run:\n\n\
         {api_prefix}fsite auth login {email}\n\n",
        project_slug = invite.project_slug,
        role = invite.role,
        email = invite.email,
        token = invite.email_login_token,
        api_prefix = api_prefix,
        git_remote_url = invite.git_remote_url,
    );
    if let Some(site) = invite.site {
        text.push_str("Project site:\n");
        text.push_str(&format!("- {} -> {}\n", site.name, site.url));
    }
    text
}

fn site_access_request_subject(site_name: &str) -> String {
    format!("Access requested for {site_name}")
}

fn site_access_request_text(request: &SiteAccessRequestEmail<'_>) -> String {
    format!(
        "{requester} verified their email and requested access to {site_name}.\n\n\
         Approve access:\n\n{approval_url}\n\n\
         Site:\n\n{site_url}\n\n\
         Ignore this email if you do not want to share the site.\n",
        requester = request.requester_email,
        site_name = request.site_name,
        approval_url = request.approval_url,
        site_url = request.site_url,
    )
}

fn first_publication_subject(site_name: &str) -> String {
    format!("{site_name} is live")
}

fn first_publication_text(site_url: &str) -> String {
    format!(
        "Your Finite Site is published.\n\n{site_url}\n\n\
         This email is your record of its first publication.\n"
    )
}

fn site_url_with_path(base_url: &str, path: &str) -> String {
    format!("{}{}", base_url.trim_end_matches('/'), path)
}

fn api_prefix(api_url: &str) -> String {
    if api_url == "https://v2.finite.chat" {
        String::new()
    } else {
        format!("FINITE_SITES_API={api_url} ")
    }
}

// ---- dev mailer ------------------------------------------------------------

pub struct DevMailer {
    outbox: finite_mail::FileOutboxMailer,
}

impl DevMailer {
    pub fn new(outbox_dir: PathBuf) -> Result<DevMailer, MailerError> {
        Ok(DevMailer {
            outbox: finite_mail::FileOutboxMailer::new(outbox_dir)?,
        })
    }
}

impl Mailer for DevMailer {
    fn send_login_link(&self, email: &str, site_name: &str, url: &str) -> Result<(), MailerError> {
        let path = self.outbox.write(
            &TextEmail {
                to: email,
                subject: &login_link_subject(site_name),
                text: &login_link_text(site_name, url),
            },
            "",
        )?;
        eprintln!(
            "dev-mail: login link for {email} -> {url} (written to {})",
            path.display()
        );
        Ok(())
    }

    fn send_email_login_token(&self, email: &str, token: &str) -> Result<(), MailerError> {
        let path = self.outbox.write(
            &TextEmail {
                to: email,
                subject: email_login_subject(),
                text: &email_login_text(email, token),
            },
            "email-login",
        )?;
        eprintln!(
            "dev-mail: email login token for {email} -> {token} (written to {})",
            path.display()
        );
        Ok(())
    }

    fn send_viewer_invite(&self, invite: &ViewerInvite<'_>) -> Result<(), MailerError> {
        self.outbox.write(
            &TextEmail {
                to: invite.email,
                subject: &viewer_invite_subject(invite.site_name),
                text: &viewer_invite_text(invite),
            },
            "viewer-invite",
        )?;
        Ok(())
    }

    fn send_project_collaborator_invite(
        &self,
        invite: &ProjectCollaboratorInvite<'_>,
    ) -> Result<(), MailerError> {
        self.outbox.write(
            &TextEmail {
                to: invite.email,
                subject: &project_collaborator_invite_subject(invite.project_slug),
                text: &project_collaborator_invite_text(invite),
            },
            "project-invite",
        )?;
        Ok(())
    }

    fn send_site_access_request(
        &self,
        request: &SiteAccessRequestEmail<'_>,
    ) -> Result<(), MailerError> {
        self.outbox.write(
            &TextEmail {
                to: request.owner_email,
                subject: &site_access_request_subject(request.site_name),
                text: &site_access_request_text(request),
            },
            "site-access-request",
        )?;
        Ok(())
    }

    fn send_first_publication(
        &self,
        email: &str,
        site_name: &str,
        site_url: &str,
    ) -> Result<(), MailerError> {
        self.outbox.write(
            &TextEmail {
                to: email,
                subject: &first_publication_subject(site_name),
                text: &first_publication_text(site_url),
            },
            "first-publication",
        )?;
        Ok(())
    }
}

// ---- http mailer (Resend via finite-mail) ------------------------------------

pub struct HttpMailer {
    resend: ResendMailer,
}

impl HttpMailer {
    pub fn new(api_key: String, from_address: String) -> HttpMailer {
        HttpMailer {
            resend: ResendMailer::new(api_key, from_address),
        }
    }
}

impl Mailer for HttpMailer {
    fn send_login_link(&self, email: &str, site_name: &str, url: &str) -> Result<(), MailerError> {
        self.resend.send_text_email(&TextEmail {
            to: email,
            subject: &login_link_subject(site_name),
            text: &login_link_text(site_name, url),
        })
    }

    fn send_email_login_token(&self, email: &str, token: &str) -> Result<(), MailerError> {
        self.resend.send_text_email(&TextEmail {
            to: email,
            subject: email_login_subject(),
            text: &email_login_text(email, token),
        })
    }

    fn send_viewer_invite(&self, invite: &ViewerInvite<'_>) -> Result<(), MailerError> {
        self.resend.send_text_email(&TextEmail {
            to: invite.email,
            subject: &viewer_invite_subject(invite.site_name),
            text: &viewer_invite_text(invite),
        })
    }

    fn send_project_collaborator_invite(
        &self,
        invite: &ProjectCollaboratorInvite<'_>,
    ) -> Result<(), MailerError> {
        self.resend.send_text_email(&TextEmail {
            to: invite.email,
            subject: &project_collaborator_invite_subject(invite.project_slug),
            text: &project_collaborator_invite_text(invite),
        })
    }

    fn send_site_access_request(
        &self,
        request: &SiteAccessRequestEmail<'_>,
    ) -> Result<(), MailerError> {
        self.resend.send_text_email(&TextEmail {
            to: request.owner_email,
            subject: &site_access_request_subject(request.site_name),
            text: &site_access_request_text(request),
        })
    }

    fn send_first_publication(
        &self,
        email: &str,
        site_name: &str,
        site_url: &str,
    ) -> Result<(), MailerError> {
        self.resend.send_text_email(&TextEmail {
            to: email,
            subject: &first_publication_subject(site_name),
            text: &first_publication_text(site_url),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finitesites_proto::dto::ProjectSiteSummary;

    #[test]
    fn mailer_kind_parsing_and_env_vars() {
        assert_eq!(MailerKind::parse("dev"), Some(MailerKind::Dev));
        assert_eq!(MailerKind::parse("resend"), Some(MailerKind::Resend));
        assert_eq!(MailerKind::parse("postmark"), None);
        assert_eq!(MailerKind::parse("sendgrid"), None);
        assert_eq!(RESEND_API_KEY_ENV_VAR, "RESEND_API_KEY");
    }

    #[test]
    fn dev_mailer_writes_login_link_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let mailer = DevMailer::new(dir.path().to_path_buf()).unwrap();
        mailer
            .send_login_link(
                "friend@example.com",
                "hello",
                "https://hello.finite.chat/_finite/auth?token=abc",
            )
            .unwrap();
        let mut entries = std::fs::read_dir(dir.path()).unwrap();
        let path = entries.next().unwrap().unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.ends_with("-friend_example_com.txt"), "{name}");
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.starts_with("To: friend@example.com\nSubject: Your link to hello\n\n"));
        assert!(contents.contains("token=abc"));
    }

    #[test]
    fn viewer_invite_leads_with_human_action_then_agent_handoff() {
        let viewer = viewer_invite_text(&ViewerInvite {
            email: "friend@example.com",
            site_name: "hello",
            site_url: "https://hello.finite.chat/",
            login_url: "https://hello.finite.chat/_finite/auth?token=abc",
        });
        assert_eq!(
            viewer,
            "hello has been shared with you.\n\n\
To view it, open this sign-in link:\n\n\
https://hello.finite.chat/_finite/auth?token=abc\n\n\
After signing in, you can view the site here:\n\n\
https://hello.finite.chat/\n\n\
The sign-in link can be reused and expires in 15 minutes. If it expires, \
open the site URL and request a fresh link for friend@example.com.\n\n\
For your agent\n\n\
If you use an agent with this site, ask it to read this email. The canonical \
site URL is:\n\n\
https://hello.finite.chat/\n\n\
Agent instructions:\n\n\
https://hello.finite.chat/llms.txt\n"
        );

        let agent_section = viewer.find("For your agent").unwrap();
        assert!(viewer.find("To view it, open this sign-in link:").unwrap() < agent_section);
        assert!(viewer.contains("ask it to read this email"));
        assert!(viewer.contains("https://hello.finite.chat/llms.txt"));

        let site = ProjectSiteSummary {
            name: "finitechat-native-mockup".to_string(),
            url: "https://finitechat-native-mockup.finite.chat/".to_string(),
            site_id: Some("site_1".to_string()),
            status: "claimed_unpublished".to_string(),
            visibility: "private".to_string(),
            active_version: None,
            branch: "main".to_string(),
            path: ".".to_string(),
            spa: false,
            created: false,
            requesting_user_shared: false,
        };
        let project = project_collaborator_invite_text(&ProjectCollaboratorInvite {
            email: "skyler@example.com",
            project_slug: "finitechat-native",
            role: "editor",
            api_url: "https://v2.finite.chat",
            git_remote_url: "https://v2.finite.chat/finitechat-native.git",
            email_login_token: "token123",
            site: Some(&site),
        });
        assert!(project.starts_with(
            "You have been invited to collaborate on finitechat-native as editor.\n\n\
If you use an agent, ask it to read the \"For your agent\" section below.\n\n\
For your agent\n\n"
        ));
        assert!(project.contains("fsite auth register --output json"));
        assert!(
            project.contains(
                "fsite auth redeem skyler@example.com token123 --link-native --output json"
            )
        );
        assert!(project.contains("fsite auth redeem skyler@example.com token123"));
        assert!(project.contains("fsite auth git finitechat-native --store --output json"));
        assert!(project.contains("fsite describe workflow edit-shared-project --output json"));
        assert!(project.contains(
            "fsite auth git finitechat-native --email skyler@example.com --store --output json"
        ));
        assert!(project.contains("git clone https://v2.finite.chat/finitechat-native.git"));
        assert!(project.contains("Project site:"));
        assert!(
            project.contains(
                "finitechat-native-mockup -> https://finitechat-native-mockup.finite.chat/"
            )
        );
    }
}
