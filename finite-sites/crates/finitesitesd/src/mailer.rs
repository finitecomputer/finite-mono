//! Outbound mail for Finite Sites. Two implementations behind one trait:
//!
//! - `DevMailer`: writes each email to a file under `DATA/outbox/` and logs
//!   the token. Selected with `--mailer dev`. Local development only;
//!   omitting `--mailer` is an error, not an implicit DevMailer.
//! - `HttpMailer`: sends through the shared `finite-mail` Resend transport.
//!   Selected with `--mailer resend`; the API key comes from the
//!   RESEND_API_KEY environment variable so secrets stay in the service env
//!   file, never in argv.
//!
//! Message text lives here (service-owned); delivery lives in `finite-mail`.
//! Site viewing no longer sends mail: viewers authenticate through the Auth
//! Gate. The remaining mail is the CLI actor path (email login tokens,
//! project collaborator invites) plus access-request and first-publication
//! notifications.

use std::path::PathBuf;

use finite_mail::{MailTransport, ResendMailer, TextEmail};
use finitesites_proto::dto::ProjectOutputSummary;

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
    fn send_email_login_token(&self, email: &str, token: &str) -> Result<(), MailerError>;
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

pub struct ProjectCollaboratorInvite<'a> {
    pub email: &'a str,
    pub project_slug: &'a str,
    pub role: &'a str,
    pub api_url: &'a str,
    pub git_remote_url: &'a str,
    pub email_login_token: &'a str,
    pub outputs: &'a [ProjectOutputSummary],
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
    if !invite.outputs.is_empty() {
        text.push_str("Project outputs:\n");
        for output in invite.outputs {
            text.push_str(&format!(
                "- {} ({}) -> {}\n",
                output.output_id, output.kind, output.site_url
            ));
        }
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

fn api_prefix(api_url: &str) -> String {
    if api_url == "https://api.finite.chat" {
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
    fn send_email_login_token(&self, email: &str, token: &str) -> Result<(), MailerError> {
        self.resend.send_text_email(&TextEmail {
            to: email,
            subject: email_login_subject(),
            text: &email_login_text(email, token),
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
    use finitesites_proto::dto::ProjectOutputSummary;

    #[test]
    fn mailer_kind_parsing_and_env_vars() {
        assert_eq!(MailerKind::parse("dev"), Some(MailerKind::Dev));
        assert_eq!(MailerKind::parse("resend"), Some(MailerKind::Resend));
        assert_eq!(MailerKind::parse("postmark"), None);
        assert_eq!(MailerKind::parse("sendgrid"), None);
        assert_eq!(RESEND_API_KEY_ENV_VAR, "RESEND_API_KEY");
    }

    #[test]
    fn dev_mailer_writes_email_login_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let mailer = DevMailer::new(dir.path().to_path_buf()).unwrap();
        mailer
            .send_email_login_token("friend@example.com", "0123abcd")
            .unwrap();
        let mut entries = std::fs::read_dir(dir.path()).unwrap();
        let path = entries.next().unwrap().unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.ends_with("-email-login.txt"), "{name}");
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(
            contents
                .starts_with("To: friend@example.com\nSubject: Your Finite Sites email login\n\n")
        );
        assert!(contents.contains("0123abcd"));
    }

    #[test]
    fn project_collaborator_invite_leads_with_agent_handoff() {
        let outputs = vec![ProjectOutputSummary {
            output_id: "mockup".to_string(),
            kind: "site".to_string(),
            output_name: "finitechat-native-mockup".to_string(),
            output_url: "https://finitechat-native-mockup.finite.chat/".to_string(),
            site_name: "finitechat-native-mockup".to_string(),
            document_name: None,
            site_id: Some("site_1".to_string()),
            site_url: "https://finitechat-native-mockup.finite.chat/".to_string(),
            status: "claimed_unpublished".to_string(),
            visibility: "private".to_string(),
            active_version: None,
            branch: "main".to_string(),
            path: ".".to_string(),
            entry: None,
            start: None,
            spa: false,
            created: false,
            requesting_user_shared: false,
        }];
        let project = project_collaborator_invite_text(&ProjectCollaboratorInvite {
            email: "skyler@example.com",
            project_slug: "finitechat-native",
            role: "editor",
            api_url: "https://api.finite.chat",
            git_remote_url: "https://git.finite.chat/finitechat-native.git",
            email_login_token: "token123",
            outputs: &outputs,
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
        assert!(project.contains("git clone https://git.finite.chat/finitechat-native.git"));
        assert!(project.contains("mockup (site)"));
    }
}
