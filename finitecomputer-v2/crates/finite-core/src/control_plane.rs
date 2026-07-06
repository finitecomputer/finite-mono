use crate::cluster::ClusterConfig;
use crate::models::{
    AuthenticateMachineTokenInput, AuthenticatedMachine, ClaimInviteInput, ConsumeOAuthStateInput,
    ControlPlaneDump, CoreExistingHostProjectImportRecord, CoreImportManifestOutput,
    CreateOAuthStateInput, EndpointAuth, EnsureGiteaCollaboratorInput, EnsureGiteaMachineUserInput,
    EnsureGiteaRepoInput, GiteaMachineAccessRecord, GiteaRepoRecord, InitOutput, InviteRecord,
    ListGiteaReposInput, ListGiteaReposOutput, ListPublishedEndpointsInput,
    ListPublishedEndpointsOutput, MachineIdInput, OAuthStateRecord, ProvisionMachineInput,
    PublishEndpointInput, PublishedEndpointRecord, PublishedEndpointRuntimeRecord,
    RenderManifestsOutput, ReservePublishedHostnameInput, RuntimeCodexStartOutput,
    RuntimeCodexStatus, RuntimeGoogleWorkspaceStatus, RuntimeImageRevisionDump,
    RuntimePublishedAppState, RuntimePublishedAppsStatusOutput, RuntimeUploadFilesInput,
    RuntimeUploadFilesOutput, SimpleOk, SiteAuthUpdateInput, UnpublishEndpointInput,
    UpdateGiteaRepoAuthInput, UpdateRuntimeProfileInput, UploadedFileRecord, WorkloadOpencode,
    WorkloadRecord, WorkloadSsh,
};
use crate::render::{
    deletion_manifest_for_removed_items, read_runtime_image_revisions, render_workload_manifest,
};
use crate::util::{
    create_machine_token, display_name_from_id, hash_machine_token, normalize_emails, now_iso,
    read_env_file, slugify, validate_emailish, write_env_file,
};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use time::OffsetDateTime;
use ureq::Error as UreqError;

#[derive(Debug, Clone)]
pub struct ControlPlanePaths {
    pub root: PathBuf,
    pub manifests_root: PathBuf,
    pub current_manifests: PathBuf,
    pub deleted_manifests: PathBuf,
    pub reconcile_root: PathBuf,
    pub reconcile_requests: PathBuf,
    pub reconcile_results: PathBuf,
    pub reconcile_logs: PathBuf,
}

pub struct ControlPlane {
    root: PathBuf,
    workspace_root: Option<PathBuf>,
    secrets_root: PathBuf,
    cluster: ClusterConfig,
    conn: Connection,
}

#[derive(Debug, serde::Deserialize)]
struct GiteaRepoApiRecord {
    name: String,
    private: bool,
    html_url: String,
    clone_url: String,
    default_branch: Option<String>,
    owner: GiteaRepoOwnerRecord,
}

#[derive(Debug, serde::Deserialize)]
struct GiteaRepoOwnerRecord {
    login: String,
}

#[derive(Debug, serde::Deserialize)]
struct GiteaUserApiRecord {
    login: String,
}

#[derive(Debug, Clone)]
struct GiteaKnownUser {
    username: String,
    email: String,
}

impl ControlPlanePaths {
    pub fn from_root(root: &Path) -> Self {
        let manifests_root = root.join("manifests");
        let reconcile_root = root.join("reconcile");
        Self {
            root: root.to_path_buf(),
            manifests_root: manifests_root.clone(),
            current_manifests: manifests_root.join("current"),
            deleted_manifests: manifests_root.join("deleted"),
            reconcile_requests: reconcile_root.join("requests"),
            reconcile_results: reconcile_root.join("results"),
            reconcile_logs: reconcile_root.join("logs"),
            reconcile_root,
        }
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        fs::create_dir_all(&self.current_manifests)?;
        fs::create_dir_all(&self.deleted_manifests)?;
        fs::create_dir_all(&self.reconcile_root)?;
        fs::create_dir_all(&self.reconcile_requests)?;
        fs::create_dir_all(&self.reconcile_results)?;
        fs::create_dir_all(&self.reconcile_logs)?;
        Ok(())
    }
}

impl ControlPlane {
    pub fn open(root: impl AsRef<Path>, workspace_root: Option<&Path>) -> Result<Self> {
        let secrets_root = env::var_os("FC_AGENT_CLUSTER_SECRETS_ROOT")
            .or_else(|| env::var_os("FC_SECRETS_ROOT"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/lib/finitecomputer/agent-cluster/secrets"));
        Self::open_with_secrets_root(root, workspace_root, secrets_root)
    }

    pub fn open_with_secrets_root(
        root: impl AsRef<Path>,
        workspace_root: Option<&Path>,
        secrets_root: PathBuf,
    ) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let paths = ControlPlanePaths::from_root(&root);
        paths.ensure()?;

        let cluster = match workspace_root {
            Some(path) => ClusterConfig::read(path)?,
            None => ClusterConfig::default(),
        };

        let mut conn = Connection::open(root.join("control-plane.sqlite"))?;
        conn.execute("PRAGMA foreign_keys = ON", [])?;
        initialize_schema(&conn)?;
        apply_schema_migrations(&conn)?;
        fill_missing_runtime_profiles(&conn, &cluster)?;
        normalize_runtime_profiles(&conn, &cluster)?;
        sync_admins(&mut conn, &cluster)?;
        conn.execute("DROP TABLE IF EXISTS setup_tasks", [])?;

        Ok(Self {
            root,
            workspace_root: workspace_root.map(PathBuf::from),
            secrets_root,
            cluster,
            conn,
        })
    }

    pub fn init_output(&self) -> Result<InitOutput> {
        let machine_count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM machines", [], |row| row.get(0))?;
        Ok(InitOutput {
            ok: true,
            bootstrapped: false,
            machine_count: machine_count as usize,
        })
    }

    fn gitea_enabled(&self) -> bool {
        self.cluster.gitea_enabled()
    }

    fn gitea_machine_username(&self, machine_id: &str) -> String {
        slugify(machine_id)
    }

    fn gitea_service_url(&self) -> Result<String> {
        if !self.gitea_enabled() {
            bail!("gitea is not enabled for this cluster");
        }
        let cluster_ip = run_kubectl_with(
            self.kubectl_command()?,
            &[
                "-n",
                self.cluster.gitea_namespace(),
                "get",
                "service",
                "gitea",
                "-o",
                "jsonpath={.spec.clusterIP}",
            ],
        )?
        .trim()
        .to_string();
        if cluster_ip.is_empty() {
            bail!("gitea service cluster IP is empty");
        }
        Ok(format!(
            "http://{}:{}",
            cluster_ip,
            self.cluster.gitea_port()
        ))
    }

    fn gitea_exec(&self, args: &[&str]) -> Result<String> {
        if !self.gitea_enabled() {
            bail!("gitea is not enabled for this cluster");
        }

        let kubectl = self.kubectl_command()?;
        run_kubectl_with(
            kubectl,
            &[
                "-n",
                self.cluster.gitea_namespace(),
                "exec",
                "deploy/gitea",
                "--",
                "gitea",
            ]
            .into_iter()
            .chain(args.iter().copied())
            .collect::<Vec<_>>(),
        )
    }

    fn gitea_user_exists(&self, username: &str) -> Result<bool> {
        Ok(self
            .gitea_known_users()?
            .iter()
            .any(|user| user.username == username))
    }

    fn gitea_username_for_email(&self, email: &str) -> Result<Option<String>> {
        let normalized = email.trim().to_lowercase();
        Ok(self.gitea_known_users()?.into_iter().find_map(|user| {
            if user.email.eq_ignore_ascii_case(&normalized) {
                Some(user.username)
            } else {
                None
            }
        }))
    }

    fn gitea_known_users(&self) -> Result<Vec<GiteaKnownUser>> {
        let output =
            self.gitea_exec(&["admin", "user", "list", "--config", "/etc/gitea/app.ini"])?;
        Ok(output
            .lines()
            .skip(1)
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let _id = parts.next()?;
                let username = parts.next()?;
                let email = parts.next()?;
                Some(GiteaKnownUser {
                    username: username.to_string(),
                    email: email.to_string(),
                })
            })
            .collect())
    }

    fn ensure_gitea_user(&self, username: &str, email: &str, display_name: &str) -> Result<()> {
        if self.gitea_user_exists(username)? {
            return Ok(());
        }

        self.gitea_exec(&[
            "admin",
            "user",
            "create",
            "--config",
            "/etc/gitea/app.ini",
            "--username",
            username,
            "--email",
            email,
            "--fullname",
            display_name,
            "--random-password",
            "--must-change-password=false",
        ])?;
        Ok(())
    }

    fn ensure_gitea_machine_user_record(
        &mut self,
        machine_id: &str,
    ) -> Result<GiteaMachineAccessRecord> {
        if !self.gitea_enabled() {
            bail!("gitea is not enabled for this cluster");
        }

        let row = self
            .conn
            .query_row(
                "SELECT owner_email, display_name FROM machines WHERE machine_id = ?",
                params![machine_id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((_owner_email, display_name)) = row else {
            bail!("machine '{}' does not exist", machine_id);
        };
        let username = self.gitea_machine_username(machine_id);
        if username.is_empty() {
            bail!(
                "machine '{}' does not produce a valid gitea username",
                machine_id
            );
        }
        let email = format!("{username}@gitea.local.invalid");
        self.ensure_gitea_user(&username, &email, &display_name)?;

        let env_path = self.machine_secret_env_path(machine_id);
        let mut env_values = read_env_file(&env_path)?;
        let current_username = env_values.get("FC_GITEA_USERNAME").cloned();
        let token = if current_username.as_deref() == Some(username.as_str()) {
            env_values
                .get("FC_GITEA_TOKEN")
                .filter(|value| !value.trim().is_empty())
                .cloned()
        } else {
            None
        };

        let token = match token {
            Some(token) => token,
            None => {
                let token_suffix = create_machine_token();
                let token_name = format!(
                    "finite-machine-{}-{}",
                    OffsetDateTime::now_utc().unix_timestamp_nanos(),
                    &token_suffix[..12]
                );
                self.gitea_exec(&[
                    "admin",
                    "user",
                    "generate-access-token",
                    "--config",
                    "/etc/gitea/app.ini",
                    "--username",
                    &username,
                    "--token-name",
                    &token_name,
                    "--raw",
                    "--scopes",
                    "all",
                ])?
                .trim()
                .to_string()
            }
        };

        env_values.insert("FC_GITEA_HOST".to_string(), self.cluster.gitea_public_url());
        env_values.insert("FC_GITEA_USERNAME".to_string(), username.clone());
        env_values.insert("FC_GITEA_TOKEN".to_string(), token.clone());
        write_env_file(&env_path, &env_values)?;

        Ok(GiteaMachineAccessRecord {
            machine_id: machine_id.to_string(),
            username: username.clone(),
            display_name,
            email,
            host_url: self.cluster.gitea_public_url(),
            clone_base_url: format!("{}/{}", self.cluster.gitea_public_url(), username),
            token,
        })
    }

    fn default_repo_org_domain(&self, machine_owner_email: Option<&str>) -> String {
        self.cluster
            .org_domain
            .clone()
            .or_else(|| email_domain(machine_owner_email))
            .unwrap_or_else(|| self.cluster.base_domain().to_string())
    }

    fn ensure_gitea_repo_auth_record(&self, machine_id: &str, repo_name: &str) -> Result<()> {
        let owner_email = self
            .conn
            .query_row(
                "SELECT owner_email FROM machines WHERE machine_id = ?",
                params![machine_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        let timestamp = now_iso();
        self.conn.execute(
            "INSERT INTO gitea_repos (machine_id, repo_name, auth_mode, auth_owner_email, auth_emails_json, auth_org_domain, created_at, updated_at)
             VALUES (?, ?, 'self', ?, '[]', NULL, ?, ?)
             ON CONFLICT(machine_id, repo_name) DO NOTHING",
            params![machine_id, repo_name, owner_email, timestamp, timestamp],
        )?;
        Ok(())
    }

    fn load_gitea_repo_auth(&self, machine_id: &str, repo_name: &str) -> Result<EndpointAuth> {
        self.ensure_gitea_repo_auth_record(machine_id, repo_name)?;
        let row = self.conn.query_row(
            "SELECT g.auth_mode, g.auth_owner_email, g.auth_emails_json, g.auth_org_domain, m.owner_email
             FROM gitea_repos g
             JOIN machines m ON m.machine_id = g.machine_id
             WHERE g.machine_id = ? AND g.repo_name = ?",
            params![machine_id, repo_name],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )?;
        let (mode, auth_owner_email, emails_json, auth_org_domain, machine_owner_email) = row;
        let emails: Vec<String> = serde_json::from_str(&emails_json).unwrap_or_default();
        Ok(match mode.as_str() {
            "self" => EndpointAuth {
                mode,
                owner_email: auth_owner_email.or(machine_owner_email),
                emails: Vec::new(),
                org_domain: None,
            },
            "emails" => EndpointAuth {
                mode,
                owner_email: None,
                emails,
                org_domain: None,
            },
            "org" => EndpointAuth {
                mode,
                owner_email: None,
                emails: Vec::new(),
                org_domain: auth_org_domain,
            },
            _ => EndpointAuth {
                mode: "public".to_string(),
                owner_email: None,
                emails: Vec::new(),
                org_domain: None,
            },
        })
    }

    fn set_gitea_repo_auth(
        &self,
        machine_id: &str,
        repo_name: &str,
        mode: &str,
        owner_email: Option<String>,
        emails_json: String,
        org_domain: Option<String>,
    ) -> Result<()> {
        let timestamp = now_iso();
        self.conn.execute(
            "INSERT INTO gitea_repos (machine_id, repo_name, auth_mode, auth_owner_email, auth_emails_json, auth_org_domain, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(machine_id, repo_name) DO UPDATE SET
               auth_mode = excluded.auth_mode,
               auth_owner_email = excluded.auth_owner_email,
               auth_emails_json = excluded.auth_emails_json,
               auth_org_domain = excluded.auth_org_domain,
               updated_at = excluded.updated_at",
            params![
                machine_id,
                repo_name,
                mode,
                owner_email,
                emails_json,
                org_domain,
                timestamp,
                timestamp,
            ],
        )?;
        Ok(())
    }

    fn gitea_repo_collaborators(
        &self,
        owner_username: &str,
        repo_name: &str,
        token: &str,
    ) -> Result<BTreeSet<String>> {
        let owner_encoded = urlencoding::encode(owner_username);
        let repo_encoded = urlencoding::encode(repo_name);
        let value = self
            .gitea_api(
                "GET",
                &format!("/api/v1/repos/{owner_encoded}/{repo_encoded}/collaborators"),
                token,
                None,
            )?
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
        let users: Vec<GiteaUserApiRecord> = serde_json::from_value(value)?;
        Ok(users.into_iter().map(|user| user.login).collect())
    }

    fn set_gitea_repo_private(
        &self,
        machine_id: &str,
        owner_username: &str,
        repo_name: &str,
        token: &str,
        private: bool,
    ) -> Result<GiteaRepoRecord> {
        let owner_encoded = urlencoding::encode(owner_username);
        let repo_encoded = urlencoding::encode(repo_name);
        let value = self
            .gitea_api(
                "PATCH",
                &format!("/api/v1/repos/{owner_encoded}/{repo_encoded}"),
                token,
                Some(json!({ "private": private })),
            )?
            .ok_or_else(|| anyhow::anyhow!("gitea returned empty response updating repository"))?;
        self.gitea_repo_record_from_value(machine_id, value)
    }

    fn machines_with_owner_emails(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT machine_id, owner_email FROM machines WHERE owner_email IS NOT NULL",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn gitea_machine_usernames_for_owner_emails(
        &mut self,
        emails: &BTreeSet<String>,
    ) -> Result<Vec<String>> {
        let machines = self.machines_with_owner_emails()?;
        let mut usernames = Vec::new();
        for (machine_id, owner_email) in machines {
            if emails.contains(owner_email.trim().to_lowercase().as_str()) {
                usernames.push(self.ensure_gitea_machine_user_record(&machine_id)?.username);
            }
        }
        usernames.sort();
        usernames.dedup();
        Ok(usernames)
    }

    fn gitea_machine_usernames_for_owner_org(&mut self, org_domain: &str) -> Result<Vec<String>> {
        let machines = self.machines_with_owner_emails()?;
        let mut usernames = Vec::new();
        for (machine_id, owner_email) in machines {
            if email_domain(Some(&owner_email)).as_deref() == Some(org_domain) {
                usernames.push(self.ensure_gitea_machine_user_record(&machine_id)?.username);
            }
        }
        usernames.sort();
        usernames.dedup();
        Ok(usernames)
    }

    fn sync_gitea_repo_policy(
        &mut self,
        machine_id: &str,
        owner_username: &str,
        token: &str,
        mut record: GiteaRepoRecord,
    ) -> Result<GiteaRepoRecord> {
        let auth = self.load_gitea_repo_auth(machine_id, &record.name)?;
        let machine_owner_email = self
            .conn
            .query_row(
                "SELECT owner_email FROM machines WHERE machine_id = ?",
                params![machine_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        let known_users = self.gitea_known_users()?;
        let mut desired_permissions = BTreeMap::<String, String>::new();
        let mut pending_emails = Vec::<String>::new();

        if let Some(owner_email) = machine_owner_email.as_deref() {
            match self.gitea_username_for_email(owner_email)? {
                Some(username) if username != owner_username => {
                    desired_permissions.insert(username, "admin".to_string());
                }
                None => pending_emails.push(owner_email.to_string()),
                _ => {}
            }
        }

        match auth.mode.as_str() {
            "emails" => {
                let auth_email_set = auth.emails.iter().cloned().collect::<BTreeSet<_>>();
                for email in &auth.emails {
                    match self.gitea_username_for_email(email)? {
                        Some(username) if username != owner_username => {
                            desired_permissions
                                .entry(username)
                                .or_insert_with(|| "write".to_string());
                        }
                        None => pending_emails.push(email.clone()),
                        _ => {}
                    }
                }
                for username in self.gitea_machine_usernames_for_owner_emails(&auth_email_set)? {
                    if username != owner_username {
                        desired_permissions
                            .entry(username)
                            .or_insert_with(|| "write".to_string());
                    }
                }
            }
            "org" => {
                let org_domain = auth.org_domain.clone().unwrap_or_else(|| {
                    self.default_repo_org_domain(machine_owner_email.as_deref())
                });
                for user in &known_users {
                    if email_domain(Some(user.email.as_str())).as_deref()
                        == Some(org_domain.as_str())
                        && user.username != owner_username
                    {
                        desired_permissions
                            .entry(user.username.clone())
                            .or_insert_with(|| "write".to_string());
                    }
                }
                for username in self.gitea_machine_usernames_for_owner_org(&org_domain)? {
                    if username != owner_username {
                        desired_permissions
                            .entry(username)
                            .or_insert_with(|| "write".to_string());
                    }
                }
            }
            _ => {}
        }

        desired_permissions.retain(|username, _| username != owner_username);

        let desired_private = auth.mode != "public";
        if record.private != desired_private {
            record = self.set_gitea_repo_private(
                machine_id,
                owner_username,
                &record.name,
                token,
                desired_private,
            )?;
        }

        let current_collaborators =
            self.gitea_repo_collaborators(owner_username, &record.name, token)?;
        let owner_encoded = urlencoding::encode(owner_username);
        let repo_encoded = urlencoding::encode(&record.name);

        for (username, permission) in &desired_permissions {
            let collaborator_encoded = urlencoding::encode(username);
            self.gitea_api(
                "PUT",
                &format!(
                    "/api/v1/repos/{owner_encoded}/{repo_encoded}/collaborators/{collaborator_encoded}"
                ),
                token,
                Some(json!({ "permission": permission })),
            )?;
        }

        for username in current_collaborators {
            if desired_permissions.contains_key(&username) {
                continue;
            }
            let collaborator_encoded = urlencoding::encode(&username);
            self.gitea_api(
                "DELETE",
                &format!(
                    "/api/v1/repos/{owner_encoded}/{repo_encoded}/collaborators/{collaborator_encoded}"
                ),
                token,
                None,
            )?;
        }

        pending_emails.sort();
        pending_emails.dedup();
        record.auth = auth;
        record.pending_emails = pending_emails;
        Ok(record)
    }

    fn gitea_api(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Option<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>> {
        let url = format!("{}{}", self.gitea_service_url()?, path);
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(20))
            .build();
        let request = agent
            .request(method, &url)
            .set("Accept", "application/json")
            .set("Authorization", &format!("token {token}"));
        let response = match body {
            Some(value) => request.send_json(value),
            None => request.call(),
        };
        match response {
            Ok(resp) => {
                if resp.status() == 204 {
                    return Ok(None);
                }
                let value: serde_json::Value = resp.into_json()?;
                Ok(Some(value))
            }
            Err(UreqError::Status(code, resp)) => {
                let body_text = resp.into_string().unwrap_or_default();
                bail!(
                    "gitea api {} {} failed with {}{}",
                    method,
                    path,
                    code,
                    if body_text.is_empty() {
                        "".to_string()
                    } else {
                        format!(": {}", body_text)
                    }
                );
            }
            Err(err) => Err(err.into()),
        }
    }

    fn gitea_repo_record_from_value(
        &self,
        machine_id: &str,
        value: serde_json::Value,
    ) -> Result<GiteaRepoRecord> {
        let parsed: GiteaRepoApiRecord = serde_json::from_value(value)?;
        Ok(GiteaRepoRecord {
            machine_id: machine_id.to_string(),
            owner: parsed.owner.login,
            name: parsed.name,
            private: parsed.private,
            html_url: parsed.html_url,
            clone_url: parsed.clone_url,
            default_branch: parsed.default_branch,
            auth: EndpointAuth::default(),
            pending_emails: Vec::new(),
        })
    }

    pub fn ensure_gitea_machine_user(
        &mut self,
        payload: &EnsureGiteaMachineUserInput,
    ) -> Result<GiteaMachineAccessRecord> {
        self.ensure_gitea_machine_user_record(&payload.machine_id)
    }

    pub fn ensure_gitea_repo(&mut self, payload: &EnsureGiteaRepoInput) -> Result<GiteaRepoRecord> {
        let access = self.ensure_gitea_machine_user_record(&payload.machine_id)?;
        let repo_name = slugify(&payload.name);
        if repo_name.is_empty() {
            bail!("repository name is empty after normalization");
        }
        let owner_encoded = urlencoding::encode(&access.username);
        let repo_encoded = urlencoding::encode(&repo_name);

        let existing = self.gitea_api(
            "GET",
            &format!("/api/v1/repos/{owner_encoded}/{repo_encoded}"),
            &access.token,
            None,
        );

        let value = match existing {
            Ok(Some(value)) => value,
            Ok(None) => bail!("gitea returned empty response for existing repository"),
            Err(err) if err.to_string().contains(" 404") => self
                .gitea_api(
                    "POST",
                    "/api/v1/user/repos",
                    &access.token,
                    Some(json!({
                        "name": repo_name,
                        "private": payload.private,
                        "auto_init": payload.auto_init,
                        "default_branch": "main",
                    })),
                )?
                .ok_or_else(|| {
                    anyhow::anyhow!("gitea returned empty response creating repository")
                })?,
            Err(err) => return Err(err),
        };

        self.ensure_gitea_repo_auth_record(&payload.machine_id, &repo_name)?;
        let record = self.sync_gitea_repo_policy(
            &payload.machine_id,
            &access.username,
            &access.token,
            self.gitea_repo_record_from_value(&payload.machine_id, value)?,
        )?;
        Ok(record)
    }

    pub fn list_gitea_repos(
        &mut self,
        payload: &ListGiteaReposInput,
    ) -> Result<ListGiteaReposOutput> {
        let access = self.ensure_gitea_machine_user_record(&payload.machine_id)?;
        let username_encoded = urlencoding::encode(&access.username);
        let value = self
            .gitea_api(
                "GET",
                &format!("/api/v1/users/{username_encoded}/repos"),
                &access.token,
                None,
            )?
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
        let items = match value {
            serde_json::Value::Array(values) => values,
            _ => bail!("gitea returned unexpected repo listing payload"),
        };
        let repos = items
            .into_iter()
            .map(|value| self.gitea_repo_record_from_value(&payload.machine_id, value))
            .collect::<Result<Vec<_>>>()?;
        let repos = repos
            .into_iter()
            .map(|repo| {
                self.ensure_gitea_repo_auth_record(&payload.machine_id, &repo.name)?;
                self.sync_gitea_repo_policy(
                    &payload.machine_id,
                    &access.username,
                    &access.token,
                    repo,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ListGiteaReposOutput {
            machine_id: payload.machine_id.clone(),
            username: access.username,
            repos,
        })
    }

    pub fn update_gitea_repo_auth(
        &mut self,
        payload: &UpdateGiteaRepoAuthInput,
    ) -> Result<GiteaRepoRecord> {
        let access = self.ensure_gitea_machine_user_record(&payload.machine_id)?;
        let repo_name = slugify(&payload.repo_name);
        if repo_name.is_empty() {
            bail!("repository name is empty after normalization");
        }
        let owner_encoded = urlencoding::encode(&access.username);
        let repo_encoded = urlencoding::encode(&repo_name);
        let value = self
            .gitea_api(
                "GET",
                &format!("/api/v1/repos/{owner_encoded}/{repo_encoded}"),
                &access.token,
                None,
            )?
            .ok_or_else(|| anyhow::anyhow!("gitea returned empty response for repository"))?;
        let machine_owner_email = self
            .conn
            .query_row(
                "SELECT owner_email FROM machines WHERE machine_id = ?",
                params![payload.machine_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        let (mode, owner_email, emails_json, org_domain) =
            auth_fields_for_gitea_repo(&self.cluster, machine_owner_email.as_deref(), payload)?;
        self.set_gitea_repo_auth(
            &payload.machine_id,
            &repo_name,
            &mode,
            owner_email,
            emails_json,
            org_domain,
        )?;
        self.sync_gitea_repo_policy(
            &payload.machine_id,
            &access.username,
            &access.token,
            self.gitea_repo_record_from_value(&payload.machine_id, value)?,
        )
    }

    pub fn ensure_gitea_collaborator(
        &mut self,
        payload: &EnsureGiteaCollaboratorInput,
    ) -> Result<SimpleOk> {
        let access = self.ensure_gitea_machine_user_record(&payload.machine_id)?;
        let owner_encoded = urlencoding::encode(&access.username);
        let repo_name = slugify(&payload.repo_name);
        let repo_encoded = urlencoding::encode(&repo_name);
        let collaborator_encoded = urlencoding::encode(&payload.collaborator);
        self.gitea_api(
            "PUT",
            &format!(
                "/api/v1/repos/{owner_encoded}/{repo_encoded}/collaborators/{collaborator_encoded}"
            ),
            &access.token,
            Some(json!({ "permission": payload.permission })),
        )?;
        Ok(SimpleOk { ok: true })
    }

    fn enqueue_reconcile(&self, reason: &str, machine_id: Option<&str>) -> Result<()> {
        let paths = ControlPlanePaths::from_root(&self.root);
        paths.ensure()?;

        let requested_at = now_iso();
        let request_id = format!(
            "{}-{}",
            slugify(reason),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        );
        let request_path = paths.reconcile_requests.join(format!("{request_id}.json"));
        let payload = json!({
            "requestedAt": requested_at,
            "reason": reason,
            "machineId": machine_id,
        });
        fs::write(request_path, serde_json::to_vec(&payload)?)?;
        Ok(())
    }

    pub fn render_manifests(&mut self) -> Result<RenderManifestsOutput> {
        let workspace_root = self
            .workspace_root
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("render-manifests requires workspace_root"))?;
        let workspace_root = workspace_root.to_path_buf();
        let paths = ControlPlanePaths::from_root(&self.root);
        paths.ensure()?;

        let rows = self
            .conn
            .prepare("SELECT machine_id FROM machines ORDER BY machine_id")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let desired_ids = rows.iter().cloned().collect::<BTreeSet<_>>();

        let mut deleted_count = 0usize;
        for entry in fs::read_dir(&paths.current_manifests)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !desired_ids.contains(stem) {
                fs::rename(&path, paths.deleted_manifests.join(entry.file_name()))?;
                deleted_count += 1;
            }
        }

        let machine_ids = desired_ids.into_iter().collect::<Vec<_>>();
        let runtime_image_revisions = read_runtime_image_revisions(&self.root)?;
        let mut rendered_count = 0usize;
        for machine_id in machine_ids {
            self.ensure_machine_api_token(&machine_id)?;
            let workload = self.conn.query_row(
                "SELECT * FROM machines WHERE machine_id = ?",
                params![machine_id],
                machine_row_to_workload,
            )?;
            let endpoints = self
                .list_published_endpoints(&ListPublishedEndpointsInput {
                    machine_id: workload.id.clone(),
                })?
                .endpoints;
            let manifest = render_workload_manifest(
                &workspace_root,
                &self.cluster,
                &workload,
                &endpoints,
                Some(&runtime_image_revisions),
            )?;
            let manifest_path = paths
                .current_manifests
                .join(format!("{}.json", workload.id));
            let stale_delete_path = paths
                .deleted_manifests
                .join(format!("{}.json", workload.id));
            let previous_manifest = if manifest_path.exists() {
                Some(serde_json::from_str::<serde_json::Value>(
                    &fs::read_to_string(&manifest_path)?,
                )?)
            } else {
                None
            };

            fs::write(
                &manifest_path,
                format!("{}\n", serde_json::to_string_pretty(&manifest)?),
            )?;

            if let Some(previous_manifest) = previous_manifest {
                if let Some(deletion_manifest) =
                    deletion_manifest_for_removed_items(&previous_manifest, &manifest)
                {
                    fs::write(
                        &stale_delete_path,
                        format!("{}\n", serde_json::to_string_pretty(&deletion_manifest)?),
                    )?;
                } else if stale_delete_path.exists() {
                    fs::remove_file(&stale_delete_path)?;
                }
            } else if stale_delete_path.exists() {
                fs::remove_file(&stale_delete_path)?;
            }
            rendered_count += 1;
        }

        Ok(RenderManifestsOutput {
            ok: true,
            rendered: rendered_count,
            deleted: deleted_count,
        })
    }

    pub fn dump_state(&self) -> Result<ControlPlaneDump> {
        let admins = self
            .conn
            .prepare("SELECT email FROM admins ORDER BY email")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let invites = self
            .conn
            .prepare(
                "SELECT machine_id, email, display_name, claim_token, created_at, claimed_at FROM invites ORDER BY machine_id",
            )?
            .query_map([], invite_row_to_record)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let workloads = self
            .conn
            .prepare("SELECT * FROM machines ORDER BY machine_id")?
            .query_map([], machine_row_to_workload)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let published_endpoints = self
            .conn
            .prepare(
                "SELECT machine_id, hostname, label, target_port, run_command, run_cwd, desired_process_state, auth_mode, auth_owner_email, auth_emails_json, auth_org_domain, created_at, updated_at FROM published_endpoints ORDER BY machine_id, hostname",
            )?
            .query_map([], published_endpoint_row_to_record_with_machine_id)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let runtime_image_revisions = read_runtime_image_revisions(&self.root)?
            .into_iter()
            .map(|(profile_id, record)| {
                (
                    profile_id,
                    RuntimeImageRevisionDump {
                        base_image: record.base_image,
                        image: record.image,
                        image_name: record.image_name,
                        image_tag: record.image_tag,
                        store_path: record.store_path,
                    },
                )
            })
            .collect();

        Ok(ControlPlaneDump {
            admins,
            invites,
            workloads,
            published_endpoints,
            runtime_image_revisions,
        })
    }

    pub fn core_import_manifest(&self, source_host_id: &str) -> Result<CoreImportManifestOutput> {
        let source_host_id = source_host_id.trim().to_lowercase();
        if source_host_id.is_empty() {
            bail!("source host id is required");
        }

        let dump = self.dump_state()?;
        let invites_by_machine = dump
            .invites
            .iter()
            .map(|invite| (invite.machine_id.as_str(), invite))
            .collect::<BTreeMap<_, _>>();
        let mut endpoints_by_machine: BTreeMap<&str, Vec<&PublishedEndpointRecord>> =
            BTreeMap::new();
        for endpoint in &dump.published_endpoints {
            endpoints_by_machine
                .entry(endpoint.machine_id.as_str())
                .or_default()
                .push(endpoint);
        }

        let records = dump
            .workloads
            .iter()
            .map(|workload| {
                let invite = invites_by_machine.get(workload.id.as_str()).copied();
                let published_app_urls = endpoints_by_machine
                    .get(workload.id.as_str())
                    .into_iter()
                    .flatten()
                    .filter(|endpoint| endpoint.status == "published")
                    .filter_map(|endpoint| published_app_url(&self.cluster, endpoint))
                    .collect::<Vec<_>>();

                CoreExistingHostProjectImportRecord {
                    source_host_id: source_host_id.clone(),
                    source_machine_id: workload.id.clone(),
                    owner_email: core_import_owner_email(workload, invite),
                    display_name: invite
                        .map(|invite| invite.display_name.trim().to_string())
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| display_name_from_id(&workload.id)),
                    hostname: non_empty_string(&workload.opencode.hostname),
                    runtime_host: Some(source_host_id.clone()),
                    runtime_status: "unknown".to_string(),
                    active_inference_profile: non_empty_string(&workload.runtime_profile),
                    hermes_available: None,
                    published_app_urls,
                    known_external_channel_participants: Vec::new(),
                    admin_visible_to_emails: dump.admins.clone(),
                }
            })
            .collect();

        Ok(CoreImportManifestOutput {
            source_host_id,
            records,
        })
    }

    pub fn provision_machine(&mut self, payload: &ProvisionMachineInput) -> Result<InviteRecord> {
        if self
            .conn
            .query_row(
                "SELECT 1 FROM machines WHERE machine_id = ?",
                params![payload.machine_id],
                |_row| Ok(()),
            )
            .optional()?
            .is_some()
        {
            bail!("machine '{}' already exists", payload.machine_id);
        }

        let runtime_profile = self
            .cluster
            .resolve_runtime_profile(payload.runtime_profile.as_deref())
            .map(|(id, _)| id)?;
        let email = validate_emailish(Some(&payload.email), "email")?;
        let timestamp = now_iso();
        self.conn.execute(
            "INSERT INTO machines (machine_id, owner, owner_email, display_name, machine_api_token_hash, namespace, runtime_profile, home_volume_size, opencode_port, hostname, project_dir, auth_mode, auth_owner_email, auth_emails_json, auth_org_domain, ssh_enabled, ssh_node_port, created_at, updated_at) VALUES (?, ?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, 'self', ?, '[]', NULL, 1, ?, ?, ?)",
            params![
                payload.machine_id,
                payload.machine_id,
                email,
                if payload.display_name.trim().is_empty() {
                    display_name_from_id(&payload.machine_id)
                } else {
                    payload.display_name.clone()
                },
                payload.machine_id,
                runtime_profile,
                payload.home_volume_size,
                i64::from(payload.port),
                payload.hostname,
                "/home/node/.hermes",
                email,
                i64::from(payload.ssh_node_port),
                payload.created_at,
                timestamp,
            ],
        )?;
        self.conn.execute(
            "INSERT INTO invites (machine_id, email, display_name, claim_token, created_at, claimed_at) VALUES (?, ?, ?, ?, ?, NULL)",
            params![
                payload.machine_id,
                email,
                if payload.display_name.trim().is_empty() {
                    display_name_from_id(&payload.machine_id)
                } else {
                    payload.display_name.clone()
                },
                payload.claim_token,
                payload.created_at,
            ],
        )?;
        self.enqueue_reconcile("provision-machine", Some(&payload.machine_id))?;

        Ok(InviteRecord {
            machine_id: payload.machine_id.clone(),
            email,
            display_name: if payload.display_name.trim().is_empty() {
                display_name_from_id(&payload.machine_id)
            } else {
                payload.display_name.clone()
            },
            claim_token: payload.claim_token.clone(),
            created_at: payload.created_at.clone(),
            claimed_at: None,
        })
    }

    pub fn deprovision_machine(&mut self, payload: &MachineIdInput) -> Result<SimpleOk> {
        let machine_exists = self
            .conn
            .query_row(
                "SELECT 1 FROM machines WHERE machine_id = ?",
                params![payload.machine_id],
                |_row| Ok(()),
            )
            .optional()?
            .is_some();
        if !machine_exists {
            bail!("machine '{}' does not exist", payload.machine_id);
        }

        self.conn.execute(
            "DELETE FROM machines WHERE machine_id = ?",
            params![payload.machine_id],
        )?;

        let secrets_dir = self.secrets_root.join(&payload.machine_id);
        match fs::remove_dir_all(&secrets_dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to remove machine secrets at {}",
                        secrets_dir.display()
                    )
                });
            }
        }

        self.enqueue_reconcile("deprovision-machine", Some(&payload.machine_id))?;
        Ok(SimpleOk { ok: true })
    }

    pub fn claim_invite(&mut self, payload: &ClaimInviteInput) -> Result<SimpleOk> {
        self.conn.execute(
            "UPDATE invites SET claimed_at = COALESCE(claimed_at, ?) WHERE claim_token = ?",
            params![payload.claimed_at, payload.claim_token],
        )?;
        Ok(SimpleOk { ok: true })
    }

    pub fn update_site_auth(&mut self, payload: &SiteAuthUpdateInput) -> Result<SimpleOk> {
        let mode = payload.mode.trim().to_lowercase();
        let timestamp = now_iso();
        let (auth_owner_email, auth_emails_json, auth_org_domain) = match mode.as_str() {
            "self" => (
                Some(validate_emailish(
                    payload.owner_email.as_deref(),
                    "ownerEmail",
                )?),
                "[]".to_string(),
                None,
            ),
            "emails" => {
                let emails = normalize_emails(&payload.emails);
                if emails.is_empty() {
                    bail!("at least one email is required for email access");
                }
                (None, serde_json::to_string(&emails)?, None)
            }
            "org" => {
                let org_domain = payload
                    .org_domain
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_lowercase())
                    .ok_or_else(|| anyhow::anyhow!("orgDomain is required for org access"))?;
                (None, "[]".to_string(), Some(org_domain))
            }
            "public" => (None, "[]".to_string(), None),
            _ => bail!("unsupported auth mode '{}'", mode),
        };

        self.conn.execute(
            "UPDATE machines SET owner_email = CASE WHEN ? = 'self' THEN ? ELSE owner_email END, auth_mode = ?, auth_owner_email = ?, auth_emails_json = ?, auth_org_domain = ?, updated_at = ? WHERE machine_id = ?",
            params![
                mode,
                auth_owner_email,
                mode,
                auth_owner_email,
                auth_emails_json,
                auth_org_domain,
                timestamp,
                payload.machine_id,
            ],
        )?;
        self.enqueue_reconcile("update-site-auth", Some(&payload.machine_id))?;
        Ok(SimpleOk { ok: true })
    }

    pub fn update_runtime_profile(
        &mut self,
        payload: &UpdateRuntimeProfileInput,
    ) -> Result<SimpleOk> {
        let runtime_profile = self
            .cluster
            .resolve_runtime_profile(Some(&payload.runtime_profile))
            .map(|(id, _)| id)?;
        self.conn.execute(
            "UPDATE machines SET runtime_profile = ?, updated_at = ? WHERE machine_id = ?",
            params![runtime_profile, now_iso(), payload.machine_id],
        )?;
        self.enqueue_reconcile("update-runtime-profile", Some(&payload.machine_id))?;
        Ok(SimpleOk { ok: true })
    }

    pub fn authenticate_machine_token(
        &self,
        payload: &AuthenticateMachineTokenInput,
    ) -> Result<AuthenticatedMachine> {
        let token_hash = hash_machine_token(&payload.token);
        let row = self.conn.query_row(
            "SELECT machine_id, owner_email, display_name FROM machines WHERE machine_api_token_hash = ?",
            params![token_hash],
            |row| {
                Ok(AuthenticatedMachine {
                    machine_id: row.get("machine_id")?,
                    owner_email: row.get("owner_email")?,
                    display_name: row.get("display_name")?,
                })
            },
        ).optional()?;
        row.ok_or_else(|| anyhow::anyhow!("invalid machine token"))
    }

    pub fn create_oauth_state(
        &mut self,
        payload: &CreateOAuthStateInput,
    ) -> Result<OAuthStateRecord> {
        let record = OAuthStateRecord {
            state: payload.state.clone(),
            provider: payload.provider.clone(),
            machine_id: payload.machine_id.clone(),
            viewer_email: payload.viewer_email.clone(),
            redirect_path: payload.redirect_path.clone(),
            created_at: payload.created_at.clone().unwrap_or_else(now_iso),
        };

        self.conn.execute(
            "INSERT INTO oauth_states (state, provider, machine_id, viewer_email, redirect_path, created_at) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(state) DO UPDATE SET provider = excluded.provider, machine_id = excluded.machine_id, viewer_email = excluded.viewer_email, redirect_path = excluded.redirect_path, created_at = excluded.created_at",
            params![
                record.state,
                record.provider,
                record.machine_id,
                record.viewer_email,
                record.redirect_path,
                record.created_at,
            ],
        )?;

        Ok(record)
    }

    pub fn consume_oauth_state(
        &mut self,
        payload: &ConsumeOAuthStateInput,
    ) -> Result<Option<OAuthStateRecord>> {
        let row = self
            .conn
            .query_row(
                "SELECT state, provider, machine_id, viewer_email, redirect_path, created_at FROM oauth_states WHERE state = ?",
                params![payload.state],
                |row| {
                    Ok(OAuthStateRecord {
                        state: row.get("state")?,
                        provider: row.get("provider")?,
                        machine_id: row.get("machine_id")?,
                        viewer_email: row.get("viewer_email")?,
                        redirect_path: row.get("redirect_path")?,
                        created_at: row.get("created_at")?,
                    })
                },
            )
            .optional()?;
        if row.is_some() {
            self.conn.execute(
                "DELETE FROM oauth_states WHERE state = ?",
                params![payload.state],
            )?;
        }
        Ok(row)
    }

    pub fn list_published_endpoints(
        &self,
        payload: &ListPublishedEndpointsInput,
    ) -> Result<ListPublishedEndpointsOutput> {
        let endpoints = self
            .conn
            .prepare(
                "SELECT hostname, label, target_port, run_command, run_cwd, desired_process_state, auth_mode, auth_owner_email, auth_emails_json, auth_org_domain, created_at, updated_at FROM published_endpoints WHERE machine_id = ? ORDER BY hostname",
            )?
            .query_map(params![payload.machine_id], published_endpoint_row_to_record)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(ListPublishedEndpointsOutput {
            machine_id: payload.machine_id.clone(),
            endpoints,
        })
    }

    pub fn runtime_published_app_statuses(
        &self,
        payload: &ListPublishedEndpointsInput,
    ) -> Result<RuntimePublishedAppsStatusOutput> {
        let endpoints = self.list_published_endpoints(payload)?.endpoints;
        let runtime = match self.runtime_exec_fc_json(
            &payload.machine_id,
            &["publish", "runtime-status", "--json"],
        ) {
            Ok(value) => value,
            Err(error) => serde_json::json!({
                "apps": [],
                "error": error.to_string(),
            }),
        };

        let runtime_by_hostname = runtime
            .get("apps")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let parsed =
                    serde_json::from_value::<RuntimePublishedAppState>(entry.clone()).ok()?;
                Some((parsed.hostname.clone(), parsed))
            })
            .collect::<std::collections::BTreeMap<_, _>>();

        let apps = endpoints
            .into_iter()
            .map(|endpoint| PublishedEndpointRuntimeRecord {
                runtime: runtime_by_hostname.get(&endpoint.hostname).cloned(),
                endpoint,
            })
            .collect();

        Ok(RuntimePublishedAppsStatusOutput {
            machine_id: payload.machine_id.clone(),
            apps,
            runtime_error: runtime
                .get("error")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
        })
    }

    pub fn reserve_published_hostname(
        &mut self,
        payload: &ReservePublishedHostnameInput,
    ) -> Result<PublishedEndpointRecord> {
        let machine = self
            .conn
            .query_row(
                "SELECT machine_id, owner_email FROM machines WHERE machine_id = ?",
                params![payload.machine_id],
                |row| {
                    Ok((
                        row.get::<_, String>("machine_id")?,
                        row.get::<_, Option<String>>("owner_email")?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("machine '{}' does not exist", payload.machine_id))?;

        let label_slug = slugify(&payload.label);
        if label_slug.is_empty() {
            bail!("label is required");
        }

        let machine_slug = slugify(&payload.machine_id);
        let stem = format!("{machine_slug}-{label_slug}");
        let mut hostname = format!("{}.{}", stem, self.cluster.base_domain());
        let existing = self.known_hostnames()?;
        let mut suffix = 2_u32;
        while existing.contains(&hostname) {
            hostname = format!("{}-{}.{}", stem, suffix, self.cluster.base_domain());
            suffix += 1;
        }

        let timestamp = now_iso();
        self.conn.execute(
            "INSERT INTO published_endpoints (hostname, machine_id, label, target_port, run_command, run_cwd, desired_process_state, auth_mode, auth_owner_email, auth_emails_json, auth_org_domain, created_at, updated_at) VALUES (?, ?, ?, NULL, NULL, NULL, 'external', 'self', ?, '[]', NULL, ?, ?)",
            params![hostname, machine.0, label_slug, machine.1, timestamp, timestamp],
        )?;

        self.conn.query_row(
            "SELECT hostname, label, target_port, run_command, run_cwd, desired_process_state, auth_mode, auth_owner_email, auth_emails_json, auth_org_domain, created_at, updated_at FROM published_endpoints WHERE hostname = ?",
            params![hostname],
            published_endpoint_row_to_record,
        ).map_err(Into::into)
    }

    pub fn publish_endpoint(
        &mut self,
        payload: &PublishEndpointInput,
    ) -> Result<PublishedEndpointRecord> {
        let machine = self
            .conn
            .query_row(
                "SELECT machine_id, owner_email FROM machines WHERE machine_id = ?",
                params![payload.machine_id],
                |row| {
                    Ok((
                        row.get::<_, String>("machine_id")?,
                        row.get::<_, Option<String>>("owner_email")?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("machine '{}' does not exist", payload.machine_id))?;

        let hostname = self.validate_published_hostname(&payload.hostname)?;
        let target_port = payload.target_port;
        if target_port == 0 {
            bail!("targetPort must be between 1 and 65535");
        }

        if payload.mode.as_deref() == Some("public")
            && payload.confirm_public.as_deref() != Some("MAKE PUBLIC")
        {
            bail!("confirmPublic must be 'MAKE PUBLIC' for public exposure");
        }

        let existing_machine_id: Option<String> = self
            .conn
            .query_row(
                "SELECT machine_id FROM published_endpoints WHERE hostname = ?",
                params![hostname],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing_machine_id) = existing_machine_id
            && existing_machine_id != payload.machine_id
        {
            bail!("hostname '{}' is already reserved", payload.hostname);
        }

        let (mode, owner_email, emails_json, org_domain) =
            auth_fields_for_endpoint(machine.1.as_deref(), payload)?;
        let label = slugify(
            payload
                .label
                .as_deref()
                .unwrap_or_else(|| hostname.split('.').next().unwrap_or_default()),
        );
        let run_command = payload
            .run_command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let run_cwd = payload
            .run_cwd
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let desired_process_state = payload.desired_process_state.clone().unwrap_or_else(|| {
            if run_command.is_some() {
                "running".to_string()
            } else {
                "external".to_string()
            }
        });
        if !matches!(
            desired_process_state.as_str(),
            "external" | "running" | "stopped"
        ) {
            bail!("desiredProcessState must be external, running, or stopped");
        }
        if run_cwd.is_some() && run_command.is_none() {
            bail!("runCwd requires runCommand");
        }

        let timestamp = now_iso();
        self.conn.execute(
            "INSERT INTO published_endpoints (hostname, machine_id, label, target_port, run_command, run_cwd, desired_process_state, auth_mode, auth_owner_email, auth_emails_json, auth_org_domain, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(hostname) DO UPDATE SET label = excluded.label, target_port = excluded.target_port, run_command = excluded.run_command, run_cwd = excluded.run_cwd, desired_process_state = excluded.desired_process_state, auth_mode = excluded.auth_mode, auth_owner_email = excluded.auth_owner_email, auth_emails_json = excluded.auth_emails_json, auth_org_domain = excluded.auth_org_domain, updated_at = excluded.updated_at",
            params![
                hostname,
                machine.0,
                label,
                i64::from(target_port),
                run_command,
                run_cwd,
                desired_process_state,
                mode,
                owner_email,
                emails_json,
                org_domain,
                timestamp,
                timestamp,
            ],
        )?;
        self.enqueue_reconcile("publish-endpoint", Some(&payload.machine_id))?;

        self.conn.query_row(
            "SELECT hostname, label, target_port, run_command, run_cwd, desired_process_state, auth_mode, auth_owner_email, auth_emails_json, auth_org_domain, created_at, updated_at FROM published_endpoints WHERE hostname = ?",
            params![hostname],
            published_endpoint_row_to_record,
        ).map_err(Into::into)
    }

    pub fn unpublish_endpoint(&mut self, payload: &UnpublishEndpointInput) -> Result<SimpleOk> {
        self.conn.execute(
            "DELETE FROM published_endpoints WHERE machine_id = ? AND hostname = ?",
            params![payload.machine_id, payload.hostname.trim().to_lowercase()],
        )?;
        self.enqueue_reconcile("unpublish-endpoint", Some(&payload.machine_id))?;
        Ok(SimpleOk { ok: true })
    }

    pub fn runtime_published_app_action(
        &mut self,
        machine_id: &str,
        hostname: &str,
        action: &str,
    ) -> Result<RuntimePublishedAppState> {
        let normalized_hostname = hostname.trim().to_lowercase();
        let row = self
            .conn
            .query_row(
                "SELECT hostname, run_command FROM published_endpoints WHERE machine_id = ? AND hostname = ?",
                params![machine_id, normalized_hostname],
                |row| Ok((row.get::<_, String>("hostname")?, row.get::<_, Option<String>>("run_command")?)),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("published endpoint '{}' does not exist", normalized_hostname))?;
        if row
            .1
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            bail!("published endpoint does not have a managed run command");
        }

        match action {
            "start" => {
                self.update_published_endpoint_process_state(
                    machine_id,
                    &normalized_hostname,
                    "running",
                )?;
            }
            "stop" => {
                self.update_published_endpoint_process_state(
                    machine_id,
                    &normalized_hostname,
                    "stopped",
                )?;
            }
            "restart" => {
                self.update_published_endpoint_process_state(
                    machine_id,
                    &normalized_hostname,
                    "running",
                )?;
            }
            _ => bail!("unsupported runtime app action '{}'", action),
        }

        let value = self.runtime_exec_fc_json(
            machine_id,
            &[
                "publish",
                &format!("runtime-{action}"),
                "--hostname",
                &normalized_hostname,
            ],
        )?;
        Ok(serde_json::from_value(value)?)
    }

    pub fn runtime_upload_files(
        &self,
        payload: &RuntimeUploadFilesInput,
    ) -> Result<RuntimeUploadFilesOutput> {
        let relpath = payload
            .destination_relpath
            .as_deref()
            .unwrap_or("uploads")
            .trim()
            .trim_matches('/');
        let relpath = if relpath.is_empty() {
            "uploads"
        } else {
            relpath
        };
        let destination_root = format!("{}/{}", self.cluster.runtime_home(), relpath);
        let uid = self.cluster.agent_uid();
        let gid = self.cluster.agent_gid();

        let staged_files = payload
            .files
            .iter()
            .map(|file_record| {
                let temp_path = PathBuf::from(&file_record.temp_path);
                if !temp_path.exists() {
                    bail!("upload temp file missing: {}", temp_path.display());
                }
                let original_name = Path::new(&file_record.name)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(ToOwned::to_owned)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("uploaded file name is required"))?;
                let bytes = fs::read(&temp_path)?;
                let size = if file_record.size > 0 {
                    file_record.size
                } else {
                    fs::metadata(&temp_path)?.len()
                };
                Ok((original_name, bytes, size))
            })
            .collect::<Result<Vec<_>>>()?;

        self.runtime_shell(
            &payload.machine_id,
            &format!(
                "mkdir -p {} && chown {}:{} {}",
                shell_quote(&destination_root),
                uid,
                gid,
                shell_quote(&destination_root)
            ),
        )?;

        let mut uploaded = Vec::new();
        for (original_name, bytes, size) in staged_files {
            let remote_path = format!("{destination_root}/{original_name}");
            self.runtime_write_file(&payload.machine_id, &remote_path, &bytes)?;
            uploaded.push(UploadedFileRecord {
                name: original_name,
                size,
                path: remote_path,
            });
        }

        if !uploaded.is_empty() {
            let chown_targets = uploaded
                .iter()
                .map(|entry| shell_quote(&entry.path))
                .collect::<Vec<_>>()
                .join(" ");
            self.runtime_shell(
                &payload.machine_id,
                &format!("chown {}:{} {}", uid, gid, chown_targets),
            )?;
        }

        Ok(RuntimeUploadFilesOutput {
            machine_id: payload.machine_id.clone(),
            destination_path: destination_root,
            files: uploaded,
        })
    }

    pub fn runtime_google_workspace_status(
        &self,
        payload: &MachineIdInput,
    ) -> Result<RuntimeGoogleWorkspaceStatus> {
        Ok(serde_json::from_value(self.runtime_exec_fc_json(
            &payload.machine_id,
            &["runtime", "google-workspace", "status"],
        )?)?)
    }

    pub fn reset_runtime_google_workspace(&self, payload: &MachineIdInput) -> Result<SimpleOk> {
        Ok(serde_json::from_value(self.runtime_exec_fc_json(
            &payload.machine_id,
            &["runtime", "google-workspace", "reset"],
        )?)?)
    }

    pub fn runtime_codex_status(&self, payload: &MachineIdInput) -> Result<RuntimeCodexStatus> {
        Ok(serde_json::from_value(self.runtime_exec_fc_json(
            &payload.machine_id,
            &["runtime", "codex", "status"],
        )?)?)
    }

    pub fn runtime_contract_diff(&self, payload: &MachineIdInput) -> Result<serde_json::Value> {
        self.runtime_exec_fc_json(&payload.machine_id, &["runtime", "contract", "diff"])
    }

    pub fn runtime_contract_reconcile(
        &self,
        payload: &MachineIdInput,
    ) -> Result<serde_json::Value> {
        self.runtime_exec_fc_json(&payload.machine_id, &["runtime", "contract", "reconcile"])
    }

    pub fn start_runtime_codex_device_auth(
        &self,
        payload: &MachineIdInput,
    ) -> Result<RuntimeCodexStartOutput> {
        Ok(serde_json::from_value(self.runtime_exec_fc_json(
            &payload.machine_id,
            &["runtime", "codex", "start"],
        )?)?)
    }

    pub fn reset_runtime_codex(&self, payload: &MachineIdInput) -> Result<SimpleOk> {
        Ok(serde_json::from_value(self.runtime_exec_fc_json(
            &payload.machine_id,
            &["runtime", "codex", "reset"],
        )?)?)
    }

    fn machine_secret_env_path(&self, machine_id: &str) -> PathBuf {
        self.secrets_root.join(machine_id).join("hermes.env")
    }

    fn ensure_machine_api_token(&mut self, machine_id: &str) -> Result<()> {
        let current_hash = self
            .conn
            .query_row(
                "SELECT machine_api_token_hash FROM machines WHERE machine_id = ?",
                params![machine_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("machine '{}' does not exist", machine_id))?;

        let env_path = self.machine_secret_env_path(machine_id);
        let mut env_values = read_env_file(&env_path)?;
        let current_token = env_values.get("FC_MACHINE_API_TOKEN").cloned();

        if let Some(current_token) = current_token
            && current_hash
                .as_deref()
                .map(|hash| hash_machine_token(&current_token) == hash)
                .unwrap_or(false)
        {
            return Ok(());
        }

        let token = create_machine_token();
        env_values.insert("FC_MACHINE_API_TOKEN".to_string(), token.clone());
        write_env_file(&env_path, &env_values)?;
        self.conn.execute(
            "UPDATE machines SET machine_api_token_hash = ?, updated_at = ? WHERE machine_id = ?",
            params![hash_machine_token(&token), now_iso(), machine_id],
        )?;
        Ok(())
    }

    fn update_published_endpoint_process_state(
        &mut self,
        machine_id: &str,
        hostname: &str,
        desired_process_state: &str,
    ) -> Result<PublishedEndpointRecord> {
        let normalized_hostname = hostname.trim().to_lowercase();
        if !matches!(desired_process_state, "external" | "running" | "stopped") {
            bail!("desiredProcessState must be external, running, or stopped");
        }

        let run_command = self
            .conn
            .query_row(
                "SELECT run_command FROM published_endpoints WHERE machine_id = ? AND hostname = ?",
                params![machine_id, normalized_hostname],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "published endpoint '{}' does not exist",
                    normalized_hostname
                )
            })?;
        if run_command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            bail!("published endpoint does not have a managed run command");
        }

        self.conn.execute(
            "UPDATE published_endpoints SET desired_process_state = ?, updated_at = ? WHERE machine_id = ? AND hostname = ?",
            params![desired_process_state, now_iso(), machine_id, normalized_hostname],
        )?;

        self.conn.query_row(
            "SELECT hostname, label, target_port, run_command, run_cwd, desired_process_state, auth_mode, auth_owner_email, auth_emails_json, auth_org_domain, created_at, updated_at FROM published_endpoints WHERE machine_id = ? AND hostname = ?",
            params![machine_id, normalized_hostname],
            published_endpoint_row_to_record,
        ).map_err(Into::into)
    }

    fn known_hostnames(&self) -> Result<BTreeSet<String>> {
        let machine_hostnames = self
            .conn
            .prepare("SELECT hostname FROM machines")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<BTreeSet<_>>>()?;
        let published_hostnames = self
            .conn
            .prepare("SELECT hostname FROM published_endpoints")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<BTreeSet<_>>>()?;
        Ok(machine_hostnames
            .into_iter()
            .chain(published_hostnames)
            .collect())
    }

    fn validate_published_hostname(&self, hostname: &str) -> Result<String> {
        let normalized = hostname.trim().to_lowercase();
        if !normalized.ends_with(&format!(".{}", self.cluster.base_domain())) {
            bail!("hostname must end with .{}", self.cluster.base_domain());
        }
        if !normalized
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '.' || ch == '-')
        {
            bail!("hostname contains invalid characters");
        }
        Ok(normalized)
    }

    // Host-side Kubernetes boundary. Keep all kubectl subprocess usage in this
    // block so it can be replaced wholesale with kube-rs later.
    fn kubectl_command(&self) -> Result<Vec<String>> {
        if let Some(value) = env::var_os("FC_KUBECTL_BIN") {
            let parts = value
                .to_string_lossy()
                .split_whitespace()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            if !parts.is_empty() {
                return Ok(parts);
            }
        }
        if env::var_os("KUBERNETES_SERVICE_HOST").is_some() && which::which("kubectl").is_ok() {
            return Ok(vec!["kubectl".to_string()]);
        }
        if which::which("k3s").is_ok() {
            return Ok(vec!["k3s".to_string(), "kubectl".to_string()]);
        }
        if which::which("kubectl").is_ok() {
            return Ok(vec!["kubectl".to_string()]);
        }
        bail!("kubectl not found (looked for 'kubectl' and 'k3s kubectl')")
    }

    fn runtime_exec(&self, machine_id: &str, command: &[&str]) -> Result<String> {
        self.runtime_exec_with_input(machine_id, command, None)
    }

    fn runtime_exec_with_input(
        &self,
        machine_id: &str,
        command: &[&str],
        input: Option<&[u8]>,
    ) -> Result<String> {
        let kubectl = self.kubectl_command()?;
        let mut completed = Command::new(&kubectl[0]);
        prepare_kubectl_command(&mut completed);
        for arg in kubectl.iter().skip(1) {
            completed.arg(arg);
        }
        let mut child = completed
            .args([
                "-n",
                machine_id,
                "exec",
                &format!("statefulset/{machine_id}"),
                "-c",
                "runtime",
                "--",
            ])
            .args(command)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(input) = input {
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("failed to open kubectl stdin"))?;
            stdin.write_all(input)?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            bail!(
                "{}",
                if !stderr.is_empty() {
                    stderr
                } else if !stdout.is_empty() {
                    stdout
                } else {
                    output.status.to_string()
                }
            );
        }
        Ok(String::from_utf8(output.stdout)?)
    }

    fn runtime_shell(&self, machine_id: &str, shell_command: &str) -> Result<String> {
        self.runtime_exec(machine_id, &["sh", "-lc", shell_command])
    }

    fn runtime_write_file(
        &self,
        machine_id: &str,
        remote_path: &str,
        bytes: &[u8],
    ) -> Result<String> {
        self.runtime_exec_with_input(
            machine_id,
            &["sh", "-lc", &format!("cat > {}", shell_quote(remote_path))],
            Some(bytes),
        )
    }

    fn runtime_exec_fc_json(&self, machine_id: &str, args: &[&str]) -> Result<serde_json::Value> {
        let mut command = vec!["finitec"];
        command.extend_from_slice(args);
        Ok(serde_json::from_str(
            &self.runtime_exec(machine_id, &command)?,
        )?)
    }
}

fn run_kubectl_with(kubectl: Vec<String>, args: &[&str]) -> Result<String> {
    if kubectl.is_empty() {
        bail!("kubectl command is empty");
    }
    let mut command = Command::new(&kubectl[0]);
    prepare_kubectl_command(&mut command);
    for arg in kubectl.iter().skip(1) {
        command.arg(arg);
    }
    let output = command.args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        bail!(
            "{}",
            if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                output.status.to_string()
            }
        );
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn initialize_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS admins (
          email TEXT PRIMARY KEY
        );

        CREATE TABLE IF NOT EXISTS machines (
          machine_id TEXT PRIMARY KEY,
          owner TEXT NOT NULL,
          owner_email TEXT,
          display_name TEXT NOT NULL,
          machine_api_token_hash TEXT,
          namespace TEXT NOT NULL,
          runtime_profile TEXT,
          home_volume_size TEXT NOT NULL,
          opencode_port INTEGER NOT NULL,
          hostname TEXT NOT NULL,
          project_dir TEXT NOT NULL,
          auth_mode TEXT NOT NULL DEFAULT 'self',
          auth_owner_email TEXT,
          auth_emails_json TEXT NOT NULL DEFAULT '[]',
          auth_org_domain TEXT,
          ssh_enabled INTEGER NOT NULL DEFAULT 1,
          ssh_node_port INTEGER,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS invites (
          machine_id TEXT PRIMARY KEY REFERENCES machines(machine_id) ON DELETE CASCADE,
          email TEXT NOT NULL,
          display_name TEXT NOT NULL,
          claim_token TEXT NOT NULL UNIQUE,
          created_at TEXT NOT NULL,
          claimed_at TEXT
        );

        CREATE TABLE IF NOT EXISTS oauth_states (
          state TEXT PRIMARY KEY,
          provider TEXT NOT NULL,
          machine_id TEXT NOT NULL REFERENCES machines(machine_id) ON DELETE CASCADE,
          viewer_email TEXT,
          redirect_path TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS published_endpoints (
          hostname TEXT PRIMARY KEY,
          machine_id TEXT NOT NULL REFERENCES machines(machine_id) ON DELETE CASCADE,
          label TEXT NOT NULL,
          target_port INTEGER,
          run_command TEXT,
          run_cwd TEXT,
          desired_process_state TEXT NOT NULL DEFAULT 'external',
          auth_mode TEXT NOT NULL DEFAULT 'self',
          auth_owner_email TEXT,
          auth_emails_json TEXT NOT NULL DEFAULT '[]',
          auth_org_domain TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS gitea_repos (
          machine_id TEXT NOT NULL REFERENCES machines(machine_id) ON DELETE CASCADE,
          repo_name TEXT NOT NULL,
          auth_mode TEXT NOT NULL DEFAULT 'self',
          auth_owner_email TEXT,
          auth_emails_json TEXT NOT NULL DEFAULT '[]',
          auth_org_domain TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          PRIMARY KEY (machine_id, repo_name)
        );
        ",
    )?;
    Ok(())
}

fn apply_schema_migrations(conn: &Connection) -> Result<()> {
    let machine_columns = table_columns(conn, "machines")?;
    if !machine_columns.contains("runtime_profile") {
        conn.execute("ALTER TABLE machines ADD COLUMN runtime_profile TEXT", [])?;
    }
    if !machine_columns.contains("machine_api_token_hash") {
        conn.execute(
            "ALTER TABLE machines ADD COLUMN machine_api_token_hash TEXT",
            [],
        )?;
    }

    let published_endpoint_columns = table_columns(conn, "published_endpoints")?;
    if !published_endpoint_columns.contains("run_command") {
        conn.execute(
            "ALTER TABLE published_endpoints ADD COLUMN run_command TEXT",
            [],
        )?;
    }
    if !published_endpoint_columns.contains("run_cwd") {
        conn.execute(
            "ALTER TABLE published_endpoints ADD COLUMN run_cwd TEXT",
            [],
        )?;
    }
    if !published_endpoint_columns.contains("desired_process_state") {
        conn.execute(
            "ALTER TABLE published_endpoints ADD COLUMN desired_process_state TEXT NOT NULL DEFAULT 'external'",
            [],
        )?;
    }

    let gitea_repo_columns = table_columns(conn, "gitea_repos")?;
    if !gitea_repo_columns.contains("auth_mode") {
        conn.execute(
            "ALTER TABLE gitea_repos ADD COLUMN auth_mode TEXT NOT NULL DEFAULT 'self'",
            [],
        )?;
    }
    if !gitea_repo_columns.contains("auth_owner_email") {
        conn.execute(
            "ALTER TABLE gitea_repos ADD COLUMN auth_owner_email TEXT",
            [],
        )?;
    }
    if !gitea_repo_columns.contains("auth_emails_json") {
        conn.execute(
            "ALTER TABLE gitea_repos ADD COLUMN auth_emails_json TEXT NOT NULL DEFAULT '[]'",
            [],
        )?;
    }
    if !gitea_repo_columns.contains("auth_org_domain") {
        conn.execute(
            "ALTER TABLE gitea_repos ADD COLUMN auth_org_domain TEXT",
            [],
        )?;
    }
    if !gitea_repo_columns.contains("created_at") {
        conn.execute(
            "ALTER TABLE gitea_repos ADD COLUMN created_at TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    if !gitea_repo_columns.contains("updated_at") {
        conn.execute(
            "ALTER TABLE gitea_repos ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    Ok(())
}

fn table_columns(conn: &Connection, table_name: &str) -> Result<BTreeSet<String>> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>("name"))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    Ok(rows)
}

fn fill_missing_runtime_profiles(conn: &Connection, cluster: &ClusterConfig) -> Result<()> {
    conn.execute(
        "UPDATE machines SET runtime_profile = ? WHERE runtime_profile IS NULL",
        params![
            cluster
                .default_runtime_profile_id()
                .unwrap_or_else(|_| "main".to_string())
        ],
    )?;
    Ok(())
}

fn normalize_runtime_profiles(conn: &Connection, cluster: &ClusterConfig) -> Result<()> {
    let mut statement = conn.prepare("SELECT machine_id, runtime_profile FROM machines")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>("machine_id")?,
            row.get::<_, Option<String>>("runtime_profile")?,
        ))
    })?;

    for row in rows {
        let (machine_id, current_profile) = row?;
        let Some(current_profile) = current_profile else {
            continue;
        };
        let resolved_profile = normalize_runtime_profile_id(cluster, &current_profile)?;
        if resolved_profile != current_profile {
            conn.execute(
                "UPDATE machines SET runtime_profile = ?, updated_at = ? WHERE machine_id = ?",
                params![resolved_profile, now_iso(), machine_id],
            )?;
        }
    }

    Ok(())
}

fn normalize_runtime_profile_id(cluster: &ClusterConfig, requested: &str) -> Result<String> {
    let trimmed = requested.trim();
    if trimmed.is_empty() || matches!(trimmed, "stable" | "canary") {
        return cluster.default_runtime_profile_id();
    }
    cluster
        .resolve_runtime_profile(Some(trimmed))
        .map(|(profile_id, _)| profile_id)
}

fn sync_admins(conn: &mut Connection, cluster: &ClusterConfig) -> Result<()> {
    let admins = cluster.dashboard_admins();
    if admins.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM admins", [])?;
    for admin in admins {
        tx.execute(
            "INSERT OR IGNORE INTO admins (email) VALUES (?)",
            params![admin],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn invite_row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<InviteRecord> {
    Ok(InviteRecord {
        machine_id: row.get("machine_id")?,
        email: row.get("email")?,
        display_name: row.get("display_name")?,
        claim_token: row.get("claim_token")?,
        created_at: row.get("created_at")?,
        claimed_at: row.get("claimed_at")?,
    })
}

fn machine_row_to_workload(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkloadRecord> {
    let auth_mode: String = row
        .get::<_, Option<String>>("auth_mode")?
        .unwrap_or_else(|| "self".to_string());
    let auth_owner_email: Option<String> = row.get("auth_owner_email")?;
    let owner_email: Option<String> = row.get("owner_email")?;
    let auth = match auth_mode.as_str() {
        "self" => EndpointAuth {
            mode: auth_mode,
            owner_email: auth_owner_email.or(owner_email.clone()),
            emails: Vec::new(),
            org_domain: None,
        },
        "emails" => EndpointAuth {
            mode: auth_mode,
            owner_email: None,
            emails: serde_json::from_str(&row.get::<_, String>("auth_emails_json")?)
                .unwrap_or_default(),
            org_domain: None,
        },
        "org" => EndpointAuth {
            mode: auth_mode,
            owner_email: None,
            emails: Vec::new(),
            org_domain: row.get("auth_org_domain")?,
        },
        _ => EndpointAuth {
            mode: auth_mode,
            owner_email: None,
            emails: Vec::new(),
            org_domain: None,
        },
    };

    Ok(WorkloadRecord {
        id: row.get("machine_id")?,
        owner: row.get("owner")?,
        owner_email,
        namespace: row.get("namespace")?,
        runtime_profile: row
            .get::<_, Option<String>>("runtime_profile")?
            .unwrap_or_else(|| "main".to_string()),
        home_volume_size: row.get("home_volume_size")?,
        opencode: WorkloadOpencode {
            port: row.get("opencode_port")?,
            hostname: row.get("hostname")?,
            project_dir: row.get("project_dir")?,
            auth,
        },
        ssh: WorkloadSsh {
            enable: row.get::<_, i64>("ssh_enabled")? != 0,
            node_port: row.get("ssh_node_port")?,
        },
    })
}

fn endpoint_auth_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EndpointAuth> {
    let auth_mode: String = row
        .get::<_, Option<String>>("auth_mode")?
        .unwrap_or_else(|| "self".to_string());
    Ok(match auth_mode.as_str() {
        "self" => EndpointAuth {
            mode: auth_mode,
            owner_email: row.get("auth_owner_email")?,
            emails: Vec::new(),
            org_domain: None,
        },
        "emails" => EndpointAuth {
            mode: auth_mode,
            owner_email: None,
            emails: serde_json::from_str(&row.get::<_, String>("auth_emails_json")?)
                .unwrap_or_default(),
            org_domain: None,
        },
        "org" => EndpointAuth {
            mode: auth_mode,
            owner_email: None,
            emails: Vec::new(),
            org_domain: row.get("auth_org_domain")?,
        },
        _ => EndpointAuth {
            mode: auth_mode,
            owner_email: None,
            emails: Vec::new(),
            org_domain: None,
        },
    })
}

fn published_endpoint_row_to_record(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<PublishedEndpointRecord> {
    Ok(PublishedEndpointRecord {
        machine_id: String::new(),
        hostname: row.get("hostname")?,
        label: row.get("label")?,
        target_port: row.get("target_port")?,
        status: if row.get::<_, Option<u16>>("target_port")?.is_some() {
            "published".to_string()
        } else {
            "reserved".to_string()
        },
        run_command: row.get("run_command")?,
        run_cwd: row.get("run_cwd")?,
        desired_process_state: row
            .get::<_, Option<String>>("desired_process_state")?
            .unwrap_or_else(|| "external".to_string()),
        auth: endpoint_auth_from_row(row)?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn published_endpoint_row_to_record_with_machine_id(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<PublishedEndpointRecord> {
    let mut record = published_endpoint_row_to_record(row)?;
    record.machine_id = row.get("machine_id")?;
    Ok(record)
}

fn core_import_owner_email(
    workload: &WorkloadRecord,
    invite: Option<&InviteRecord>,
) -> Option<String> {
    normalize_core_import_email(workload.opencode.auth.owner_email.as_deref())
        .or_else(|| normalize_core_import_email(workload.owner_email.as_deref()))
        .or_else(|| {
            if workload.opencode.auth.mode == "emails" {
                workload
                    .opencode
                    .auth
                    .emails
                    .iter()
                    .find_map(|email| normalize_core_import_email(Some(email)))
            } else {
                None
            }
        })
        .or_else(|| normalize_core_import_email(invite.map(|invite| invite.email.as_str())))
}

fn normalize_core_import_email(value: Option<&str>) -> Option<String> {
    let email = value?.trim().to_lowercase();
    if email.is_empty() { None } else { Some(email) }
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn published_app_url(
    cluster: &ClusterConfig,
    endpoint: &PublishedEndpointRecord,
) -> Option<String> {
    let hostname = endpoint.hostname.trim();
    if hostname.is_empty() {
        return None;
    }

    Some(format!("{}://{}", cluster.published_url_scheme(), hostname))
}

fn auth_fields_for_endpoint(
    machine_owner_email: Option<&str>,
    payload: &PublishEndpointInput,
) -> Result<(String, Option<String>, String, Option<String>)> {
    let mode = payload
        .mode
        .as_deref()
        .unwrap_or("self")
        .trim()
        .to_lowercase();
    let mut owner_email = None;
    let mut emails_json = "[]".to_string();
    let mut org_domain = None;

    match mode.as_str() {
        "self" => {
            owner_email = Some(validate_emailish(
                payload.owner_email.as_deref().or(machine_owner_email),
                "ownerEmail",
            )?);
        }
        "emails" => {
            let emails = normalize_emails(&payload.emails);
            if emails.is_empty() {
                bail!("at least one email is required for email access");
            }
            emails_json = serde_json::to_string(&emails)?;
        }
        "org" => {
            org_domain = payload
                .org_domain
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_lowercase());
            if org_domain.is_none() {
                bail!("orgDomain is required for org access");
            }
        }
        "public" => {}
        _ => bail!("unsupported auth mode '{}'", mode),
    }

    Ok((mode, owner_email, emails_json, org_domain))
}

fn auth_fields_for_gitea_repo(
    cluster: &ClusterConfig,
    machine_owner_email: Option<&str>,
    payload: &UpdateGiteaRepoAuthInput,
) -> Result<(String, Option<String>, String, Option<String>)> {
    let mode = payload.mode.trim().to_lowercase();
    let mut owner_email = None;
    let mut emails_json = "[]".to_string();
    let mut org_domain = None;

    match mode.as_str() {
        "self" => {
            owner_email = Some(validate_emailish(
                payload.owner_email.as_deref().or(machine_owner_email),
                "ownerEmail",
            )?);
        }
        "emails" => {
            let emails = normalize_emails(&payload.emails);
            if emails.is_empty() {
                bail!("at least one email is required for email access");
            }
            emails_json = serde_json::to_string(&emails)?;
        }
        "org" => {
            org_domain = payload
                .org_domain
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_lowercase())
                .or_else(|| {
                    cluster
                        .org_domain
                        .clone()
                        .or_else(|| email_domain(machine_owner_email))
                        .or_else(|| Some(cluster.base_domain().to_string()))
                });
            if org_domain.is_none() {
                bail!("orgDomain is required for org access");
            }
        }
        "public" => {
            if payload.confirm_public.as_deref() != Some("MAKE PUBLIC") {
                bail!("confirmPublic must be 'MAKE PUBLIC' for public access");
            }
        }
        _ => bail!("unsupported auth mode '{}'", mode),
    }

    Ok((mode, owner_email, emails_json, org_domain))
}

fn email_domain(email: Option<&str>) -> Option<String> {
    let email = email?.trim().to_lowercase();
    let (_, domain) = email.split_once('@')?;
    let domain = domain.trim();
    if domain.is_empty() {
        None
    } else {
        Some(domain.to_string())
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn prepare_kubectl_command(command: &mut Command) {
    // Dashboard runs with a read-only root filesystem, so kubectl discovery/cache state
    // must live under /tmp when invoked from inside the pod.
    command
        .env("HOME", "/tmp")
        .env("XDG_CACHE_HOME", "/tmp/.cache")
        .env("KUBECACHEDIR", "/tmp/.kube/cache");
}

#[cfg(test)]
mod tests {
    use super::{ControlPlane, auth_fields_for_gitea_repo};
    use crate::cluster::ClusterConfig;
    use crate::models::{
        ConsumeOAuthStateInput, CreateOAuthStateInput, ListPublishedEndpointsInput, MachineIdInput,
        ProvisionMachineInput, PublishEndpointInput, ReservePublishedHostnameInput,
        RuntimeUploadFileInput, RuntimeUploadFilesInput, UpdateGiteaRepoAuthInput,
        UpdateRuntimeProfileInput,
    };
    use crate::util::bounded_kube_name;
    use std::fs;
    use tempfile::tempdir;

    fn write_cluster(workspace_root: &std::path::Path) {
        write_cluster_for_domain(workspace_root, "finite.vip", false);
    }

    fn write_oauth_cluster(workspace_root: &std::path::Path, base_domain: &str) {
        write_cluster_for_domain(workspace_root, base_domain, true);
    }

    fn write_cluster_for_domain(
        workspace_root: &std::path::Path,
        base_domain: &str,
        oauth_enabled: bool,
    ) {
        let path = workspace_root.join("agent-cluster");
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("cluster.json"),
            format!(
                r#"{{
              "base_domain": "{base_domain}",
              "default_runtime_profile": "main",
              "runtime_profiles": {{
                "main": {{
                  "label": "Hermes Runtime",
                  "feature_set": "hermes-local"
                }}
              }},
              "dashboard": {{
                "admins": ["paul@finite.vip"]
              }},
              "oauth2_proxy": {{
                "enabled": {oauth_enabled}
              }}
            }}"#
            ),
        )
        .unwrap();
    }

    fn write_runtime_seed(workspace_root: &std::path::Path) {
        let seed_root = workspace_root.join("agent-cluster/runtime-seed");
        fs::create_dir_all(seed_root.join("hermes/memories")).unwrap();
        fs::write(seed_root.join("FINITE.md"), "# FINITE\n").unwrap();
        fs::write(seed_root.join("hermes/config.yaml"), "models: {}\n").unwrap();
        fs::write(seed_root.join("hermes/SOUL.md"), "# SOUL\n").unwrap();
        fs::write(seed_root.join("hermes/memories/MEMORY.md"), "# MEMORY\n").unwrap();
        fs::write(seed_root.join("hermes/memories/USER.md"), "# USER\n").unwrap();
    }

    #[test]
    fn repo_org_auth_defaults_to_owner_email_domain_when_cluster_org_missing() {
        let cluster = ClusterConfig {
            base_domain: Some("smoke.finite.computer".to_string()),
            ..ClusterConfig::default()
        };
        let payload = UpdateGiteaRepoAuthInput {
            machine_id: "paul-smoke".to_string(),
            repo_name: "user-skills".to_string(),
            mode: "org".to_string(),
            owner_email: None,
            emails: Vec::new(),
            org_domain: None,
            confirm_public: None,
        };

        let (mode, owner_email, emails_json, org_domain) =
            auth_fields_for_gitea_repo(&cluster, Some("paul@finite.vip"), &payload).unwrap();
        assert_eq!(mode, "org");
        assert_eq!(owner_email, None);
        assert_eq!(emails_json, "[]");
        assert_eq!(org_domain.as_deref(), Some("finite.vip"));
    }

    #[test]
    fn repo_public_auth_requires_explicit_confirmation() {
        let cluster = ClusterConfig {
            base_domain: Some("finite.vip".to_string()),
            ..ClusterConfig::default()
        };
        let payload = UpdateGiteaRepoAuthInput {
            machine_id: "paul-finite-2".to_string(),
            repo_name: "user-skills".to_string(),
            mode: "public".to_string(),
            owner_email: None,
            emails: Vec::new(),
            org_domain: None,
            confirm_public: None,
        };
        let error = auth_fields_for_gitea_repo(&cluster, Some("paul@finite.vip"), &payload)
            .unwrap_err()
            .to_string();
        assert!(error.contains("MAKE PUBLIC"));
    }

    #[test]
    fn provision_and_dump_state_round_trips() {
        let workspace_root = tempdir().unwrap();
        let control_plane_root = tempdir().unwrap();
        write_cluster(workspace_root.path());
        let mut control_plane =
            ControlPlane::open(control_plane_root.path(), Some(workspace_root.path())).unwrap();

        let invite = control_plane
            .provision_machine(&ProvisionMachineInput {
                machine_id: "paul-finite-2".to_string(),
                display_name: "Paul 2".to_string(),
                email: "paul@finite.vip".to_string(),
                runtime_profile: None,
                home_volume_size: "20Gi".to_string(),
                hostname: "paul2-opencode.finite.vip".to_string(),
                port: 4101,
                ssh_node_port: 32222,
                claim_token: "claim-token".to_string(),
                created_at: "2026-04-09T00:00:00Z".to_string(),
            })
            .unwrap();
        assert_eq!(invite.email, "paul@finite.vip");

        let dump = control_plane.dump_state().unwrap();
        assert_eq!(dump.admins, vec!["paul@finite.vip".to_string()]);
        assert_eq!(dump.workloads.len(), 1);
        assert_eq!(dump.workloads[0].runtime_profile, "main");
        assert_eq!(dump.invites.len(), 1);
    }

    #[test]
    fn core_import_manifest_projects_current_host_state_without_claiming() {
        let workspace_root = tempdir().unwrap();
        let control_plane_root = tempdir().unwrap();
        write_cluster(workspace_root.path());
        let mut control_plane =
            ControlPlane::open(control_plane_root.path(), Some(workspace_root.path())).unwrap();
        control_plane
            .provision_machine(&ProvisionMachineInput {
                machine_id: "paul-finite-2".to_string(),
                display_name: "Paul 2".to_string(),
                email: "Paul@Finite.Vip".to_string(),
                runtime_profile: None,
                home_volume_size: "20Gi".to_string(),
                hostname: "paul2-opencode.finite.vip".to_string(),
                port: 4101,
                ssh_node_port: 32222,
                claim_token: "claim-token".to_string(),
                created_at: "2026-04-09T00:00:00Z".to_string(),
            })
            .unwrap();
        let reserved = control_plane
            .reserve_published_hostname(&ReservePublishedHostnameInput {
                machine_id: "paul-finite-2".to_string(),
                label: "demo".to_string(),
            })
            .unwrap();
        control_plane
            .publish_endpoint(&PublishEndpointInput {
                machine_id: "paul-finite-2".to_string(),
                hostname: reserved.hostname.clone(),
                target_port: 5173,
                label: Some("demo".to_string()),
                mode: Some("self".to_string()),
                owner_email: None,
                emails: Vec::new(),
                org_domain: None,
                confirm_public: None,
                run_command: None,
                run_cwd: None,
                desired_process_state: None,
            })
            .unwrap();

        let manifest = control_plane.core_import_manifest("box1").unwrap();

        assert_eq!(manifest.source_host_id, "box1");
        assert_eq!(manifest.records.len(), 1);
        let record = &manifest.records[0];
        assert_eq!(record.source_host_id, "box1");
        assert_eq!(record.source_machine_id, "paul-finite-2");
        assert_eq!(record.owner_email.as_deref(), Some("paul@finite.vip"));
        assert_eq!(record.display_name, "Paul 2");
        assert_eq!(
            record.hostname.as_deref(),
            Some("paul2-opencode.finite.vip")
        );
        assert_eq!(record.runtime_host.as_deref(), Some("box1"));
        assert_eq!(record.runtime_status, "unknown");
        assert_eq!(record.active_inference_profile.as_deref(), Some("main"));
        assert_eq!(
            record.published_app_urls,
            vec![format!("http://{}", reserved.hostname)]
        );
        assert_eq!(
            record.admin_visible_to_emails,
            vec!["paul@finite.vip".to_string()]
        );
    }

    #[test]
    fn core_import_manifest_requires_explicit_source_host_id() {
        let workspace_root = tempdir().unwrap();
        let control_plane_root = tempdir().unwrap();
        write_cluster(workspace_root.path());
        let control_plane =
            ControlPlane::open(control_plane_root.path(), Some(workspace_root.path())).unwrap();

        let error = control_plane
            .core_import_manifest(" ")
            .unwrap_err()
            .to_string();

        assert!(error.contains("source host id is required"));
    }

    #[test]
    fn reserve_and_publish_endpoint() {
        let workspace_root = tempdir().unwrap();
        let control_plane_root = tempdir().unwrap();
        write_cluster(workspace_root.path());
        let mut control_plane =
            ControlPlane::open(control_plane_root.path(), Some(workspace_root.path())).unwrap();
        control_plane
            .provision_machine(&ProvisionMachineInput {
                machine_id: "paul-finite-2".to_string(),
                display_name: "Paul 2".to_string(),
                email: "paul@finite.vip".to_string(),
                runtime_profile: None,
                home_volume_size: "20Gi".to_string(),
                hostname: "paul2-opencode.finite.vip".to_string(),
                port: 4101,
                ssh_node_port: 32222,
                claim_token: "claim-token".to_string(),
                created_at: "2026-04-09T00:00:00Z".to_string(),
            })
            .unwrap();

        let reserved = control_plane
            .reserve_published_hostname(&ReservePublishedHostnameInput {
                machine_id: "paul-finite-2".to_string(),
                label: "skills".to_string(),
            })
            .unwrap();
        assert_eq!(reserved.status, "reserved");
        assert!(reserved.hostname.ends_with(".finite.vip"));

        let published = control_plane
            .publish_endpoint(&PublishEndpointInput {
                machine_id: "paul-finite-2".to_string(),
                hostname: reserved.hostname.clone(),
                target_port: 4243,
                label: Some("skills".to_string()),
                mode: Some("self".to_string()),
                owner_email: None,
                emails: Vec::new(),
                org_domain: None,
                confirm_public: None,
                run_command: Some("python3 -m http.server 4243".to_string()),
                run_cwd: Some("/home/node/dev/skills-site".to_string()),
                desired_process_state: None,
            })
            .unwrap();
        assert_eq!(published.status, "published");
        assert_eq!(published.desired_process_state, "running");

        let list = control_plane
            .list_published_endpoints(&ListPublishedEndpointsInput {
                machine_id: "paul-finite-2".to_string(),
            })
            .unwrap();
        assert_eq!(list.endpoints.len(), 1);
        assert_eq!(
            list.endpoints[0].run_cwd.as_deref(),
            Some("/home/node/dev/skills-site")
        );
    }

    #[test]
    fn public_publish_requires_confirmation() {
        let workspace_root = tempdir().unwrap();
        let control_plane_root = tempdir().unwrap();
        write_cluster(workspace_root.path());
        let mut control_plane =
            ControlPlane::open(control_plane_root.path(), Some(workspace_root.path())).unwrap();
        control_plane
            .provision_machine(&ProvisionMachineInput {
                machine_id: "paul-finite-2".to_string(),
                display_name: "Paul 2".to_string(),
                email: "paul@finite.vip".to_string(),
                runtime_profile: None,
                home_volume_size: "20Gi".to_string(),
                hostname: "paul2-opencode.finite.vip".to_string(),
                port: 4101,
                ssh_node_port: 32222,
                claim_token: "claim-token".to_string(),
                created_at: "2026-04-09T00:00:00Z".to_string(),
            })
            .unwrap();

        let error = control_plane
            .publish_endpoint(&PublishEndpointInput {
                machine_id: "paul-finite-2".to_string(),
                hostname: "paul-finite-2-public.finite.vip".to_string(),
                target_port: 3000,
                label: Some("public".to_string()),
                mode: Some("public".to_string()),
                owner_email: None,
                emails: Vec::new(),
                org_domain: None,
                confirm_public: None,
                run_command: None,
                run_cwd: None,
                desired_process_state: None,
            })
            .unwrap_err();
        assert!(error.to_string().contains("MAKE PUBLIC"));
    }

    #[test]
    fn render_manifests_emits_delete_manifest_when_endpoint_auth_resources_are_removed() {
        let workspace_root = tempdir().unwrap();
        let control_plane_root = tempdir().unwrap();
        let secrets_root = tempdir().unwrap();
        write_oauth_cluster(workspace_root.path(), "finite.vip");
        write_runtime_seed(workspace_root.path());
        let mut control_plane = ControlPlane::open_with_secrets_root(
            control_plane_root.path(),
            Some(workspace_root.path()),
            secrets_root.path().to_path_buf(),
        )
        .unwrap();
        control_plane
            .provision_machine(&ProvisionMachineInput {
                machine_id: "skyler-finite".to_string(),
                display_name: "Skyler".to_string(),
                email: "skyler@finite.vip".to_string(),
                runtime_profile: None,
                home_volume_size: "20Gi".to_string(),
                hostname: "skyler-opencode.finite.vip".to_string(),
                port: 4101,
                ssh_node_port: 32222,
                claim_token: "claim-token".to_string(),
                created_at: "2026-04-09T00:00:00Z".to_string(),
            })
            .unwrap();

        let hostname = "skyler-finite-john1.finite.vip";
        control_plane
            .publish_endpoint(&PublishEndpointInput {
                machine_id: "skyler-finite".to_string(),
                hostname: hostname.to_string(),
                target_port: 7410,
                label: None,
                mode: Some("emails".to_string()),
                owner_email: None,
                emails: vec!["paul@finite.vip".to_string()],
                org_domain: None,
                confirm_public: None,
                run_command: None,
                run_cwd: None,
                desired_process_state: None,
            })
            .unwrap();
        control_plane.render_manifests().unwrap();

        control_plane
            .publish_endpoint(&PublishEndpointInput {
                machine_id: "skyler-finite".to_string(),
                hostname: hostname.to_string(),
                target_port: 7410,
                label: None,
                mode: Some("public".to_string()),
                owner_email: None,
                emails: Vec::new(),
                org_domain: None,
                confirm_public: Some("MAKE PUBLIC".to_string()),
                run_command: None,
                run_cwd: None,
                desired_process_state: None,
            })
            .unwrap();
        control_plane.render_manifests().unwrap();

        let delete_manifest_path = control_plane_root
            .path()
            .join("manifests/deleted/skyler-finite.json");
        let delete_manifest = serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(delete_manifest_path).unwrap(),
        )
        .unwrap();
        let removed = delete_manifest["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                (
                    item["kind"].as_str().unwrap().to_string(),
                    item["metadata"]["name"].as_str().unwrap().to_string(),
                )
            })
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(
            removed,
            std::collections::BTreeSet::from([
                (
                    "Service".to_string(),
                    bounded_kube_name(hostname, "oauth2-proxy"),
                ),
                (
                    "Middleware".to_string(),
                    bounded_kube_name(hostname, "oauth2-auth"),
                ),
                (
                    "Middleware".to_string(),
                    bounded_kube_name(hostname, "oauth2-errors"),
                ),
            ])
        );
    }

    #[test]
    fn render_manifests_bounds_long_published_endpoint_resource_names() {
        let workspace_root = tempdir().unwrap();
        let control_plane_root = tempdir().unwrap();
        let secrets_root = tempdir().unwrap();
        write_oauth_cluster(workspace_root.path(), "trf.finite.computer");
        write_runtime_seed(workspace_root.path());
        let mut control_plane = ControlPlane::open_with_secrets_root(
            control_plane_root.path(),
            Some(workspace_root.path()),
            secrets_root.path().to_path_buf(),
        )
        .unwrap();
        control_plane
            .provision_machine(&ProvisionMachineInput {
                machine_id: "skyler-finite".to_string(),
                display_name: "Skyler".to_string(),
                email: "skyler@finite.vip".to_string(),
                runtime_profile: None,
                home_volume_size: "20Gi".to_string(),
                hostname: "skyler-opencode.trf.finite.computer".to_string(),
                port: 4101,
                ssh_node_port: 32222,
                claim_token: "claim-token".to_string(),
                created_at: "2026-04-09T00:00:00Z".to_string(),
            })
            .unwrap();

        let hostname = "jeremy-ani-art-academies-dashboard.trf.finite.computer";
        control_plane
            .publish_endpoint(&PublishEndpointInput {
                machine_id: "skyler-finite".to_string(),
                hostname: hostname.to_string(),
                target_port: 7410,
                label: None,
                mode: Some("self".to_string()),
                owner_email: None,
                emails: Vec::new(),
                org_domain: None,
                confirm_public: None,
                run_command: None,
                run_cwd: None,
                desired_process_state: None,
            })
            .unwrap();
        control_plane.render_manifests().unwrap();

        let manifest_path = control_plane_root
            .path()
            .join("manifests/current/skyler-finite.json");
        let manifest =
            serde_json::from_str::<serde_json::Value>(&fs::read_to_string(manifest_path).unwrap())
                .unwrap();
        let expected_names = std::collections::BTreeSet::from([
            bounded_kube_name(hostname, ""),
            bounded_kube_name(hostname, "svc"),
            bounded_kube_name(hostname, "oauth2-proxy"),
            bounded_kube_name(hostname, "oauth2-auth"),
            bounded_kube_name(hostname, "oauth2-errors"),
        ]);
        let found_names = manifest["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| {
                let kind = item["kind"].as_str()?;
                let name = item["metadata"]["name"].as_str()?;
                if matches!(kind, "Service" | "Middleware" | "IngressRoute")
                    && expected_names.contains(name)
                {
                    Some(name.to_string())
                } else {
                    None
                }
            })
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(found_names, expected_names);
        for name in found_names {
            assert!(name.len() <= 63);
        }
    }

    #[test]
    fn update_runtime_profile_validates_cluster_profiles() {
        let workspace_root = tempdir().unwrap();
        let control_plane_root = tempdir().unwrap();
        write_cluster(workspace_root.path());
        let mut control_plane =
            ControlPlane::open(control_plane_root.path(), Some(workspace_root.path())).unwrap();
        control_plane
            .provision_machine(&ProvisionMachineInput {
                machine_id: "paul-finite-2".to_string(),
                display_name: "Paul 2".to_string(),
                email: "paul@finite.vip".to_string(),
                runtime_profile: None,
                home_volume_size: "20Gi".to_string(),
                hostname: "paul2-opencode.finite.vip".to_string(),
                port: 4101,
                ssh_node_port: 32222,
                claim_token: "claim-token".to_string(),
                created_at: "2026-04-09T00:00:00Z".to_string(),
            })
            .unwrap();

        control_plane
            .update_runtime_profile(&UpdateRuntimeProfileInput {
                machine_id: "paul-finite-2".to_string(),
                runtime_profile: "main".to_string(),
            })
            .unwrap();

        let dump = control_plane.dump_state().unwrap();
        assert_eq!(dump.workloads[0].runtime_profile, "main");
    }

    #[test]
    fn render_manifests_writes_current_manifest_and_machine_token() {
        let workspace_root = tempdir().unwrap();
        let control_plane_root = tempdir().unwrap();
        let secrets_root = tempdir().unwrap();
        write_cluster(workspace_root.path());
        write_runtime_seed(workspace_root.path());
        let mut control_plane = ControlPlane::open_with_secrets_root(
            control_plane_root.path(),
            Some(workspace_root.path()),
            secrets_root.path().to_path_buf(),
        )
        .unwrap();
        control_plane
            .provision_machine(&ProvisionMachineInput {
                machine_id: "paul-finite-2".to_string(),
                display_name: "Paul 2".to_string(),
                email: "paul@finite.vip".to_string(),
                runtime_profile: None,
                home_volume_size: "20Gi".to_string(),
                hostname: "paul2-opencode.finite.vip".to_string(),
                port: 4101,
                ssh_node_port: 32222,
                claim_token: "claim-token".to_string(),
                created_at: "2026-04-09T00:00:00Z".to_string(),
            })
            .unwrap();

        let result = control_plane.render_manifests().unwrap();
        assert_eq!(result.rendered, 1);

        let manifest_path = control_plane_root
            .path()
            .join("manifests/current/paul-finite-2.json");
        let manifest =
            serde_json::from_str::<serde_json::Value>(&fs::read_to_string(manifest_path).unwrap())
                .unwrap();
        assert_eq!(manifest["kind"], "List");
        assert!(manifest["items"].as_array().unwrap().len() >= 5);

        let env_file = secrets_root.path().join("paul-finite-2/hermes.env");
        let env_text = fs::read_to_string(env_file).unwrap();
        assert!(env_text.contains("FC_MACHINE_API_TOKEN="));
    }

    #[test]
    fn deprovision_machine_cleans_db_state_and_moves_manifest_to_deleted() {
        let workspace_root = tempdir().unwrap();
        let control_plane_root = tempdir().unwrap();
        let secrets_root = tempdir().unwrap();
        write_cluster(workspace_root.path());
        write_runtime_seed(workspace_root.path());
        let mut control_plane = ControlPlane::open_with_secrets_root(
            control_plane_root.path(),
            Some(workspace_root.path()),
            secrets_root.path().to_path_buf(),
        )
        .unwrap();
        control_plane
            .provision_machine(&ProvisionMachineInput {
                machine_id: "paul-finite-2".to_string(),
                display_name: "Paul 2".to_string(),
                email: "paul@finite.vip".to_string(),
                runtime_profile: None,
                home_volume_size: "20Gi".to_string(),
                hostname: "paul2-opencode.finite.vip".to_string(),
                port: 4101,
                ssh_node_port: 32222,
                claim_token: "claim-token".to_string(),
                created_at: "2026-04-09T00:00:00Z".to_string(),
            })
            .unwrap();
        control_plane.render_manifests().unwrap();
        let current_manifest_path = control_plane_root
            .path()
            .join("manifests/current/paul-finite-2.json");
        assert!(current_manifest_path.exists());
        let secrets_dir = secrets_root.path().join("paul-finite-2");
        assert!(secrets_dir.exists());

        control_plane
            .deprovision_machine(&MachineIdInput {
                machine_id: "paul-finite-2".to_string(),
            })
            .unwrap();

        let dump = control_plane.dump_state().unwrap();
        assert!(dump.workloads.is_empty());
        assert!(dump.invites.is_empty());
        assert!(!secrets_dir.exists());

        let result = control_plane.render_manifests().unwrap();
        assert_eq!(result.rendered, 0);
        assert_eq!(result.deleted, 1);
        let deleted_manifest_path = control_plane_root
            .path()
            .join("manifests/deleted/paul-finite-2.json");
        assert!(!current_manifest_path.exists());
        assert!(deleted_manifest_path.exists());
    }

    #[test]
    fn oauth_state_round_trips_and_consumes() {
        let workspace_root = tempdir().unwrap();
        let control_plane_root = tempdir().unwrap();
        write_cluster(workspace_root.path());
        let mut control_plane =
            ControlPlane::open(control_plane_root.path(), Some(workspace_root.path())).unwrap();
        control_plane
            .provision_machine(&ProvisionMachineInput {
                machine_id: "paul-finite-2".to_string(),
                display_name: "Paul 2".to_string(),
                email: "paul@finite.vip".to_string(),
                runtime_profile: None,
                home_volume_size: "20Gi".to_string(),
                hostname: "paul2-opencode.finite.vip".to_string(),
                port: 4101,
                ssh_node_port: 32222,
                claim_token: "claim-token".to_string(),
                created_at: "2026-04-09T00:00:00Z".to_string(),
            })
            .unwrap();

        let created = control_plane
            .create_oauth_state(&CreateOAuthStateInput {
                state: "abc".to_string(),
                provider: "google_workspace".to_string(),
                machine_id: "paul-finite-2".to_string(),
                viewer_email: Some("paul@finite.vip".to_string()),
                redirect_path: "/dashboard/machines/paul-finite-2".to_string(),
                created_at: Some("2026-04-09T00:00:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(created.state, "abc");

        let consumed = control_plane
            .consume_oauth_state(&ConsumeOAuthStateInput {
                state: "abc".to_string(),
            })
            .unwrap()
            .unwrap();
        assert_eq!(consumed.provider, "google_workspace");

        let missing = control_plane
            .consume_oauth_state(&ConsumeOAuthStateInput {
                state: "abc".to_string(),
            })
            .unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn shell_quote_handles_single_quotes() {
        assert_eq!(super::shell_quote("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn runtime_upload_files_requires_real_inputs() {
        let workspace_root = tempdir().unwrap();
        let control_plane_root = tempdir().unwrap();
        write_cluster(workspace_root.path());
        let mut control_plane =
            ControlPlane::open(control_plane_root.path(), Some(workspace_root.path())).unwrap();
        control_plane
            .provision_machine(&ProvisionMachineInput {
                machine_id: "paul-finite-2".to_string(),
                display_name: "Paul 2".to_string(),
                email: "paul@finite.vip".to_string(),
                runtime_profile: None,
                home_volume_size: "20Gi".to_string(),
                hostname: "paul2-opencode.finite.vip".to_string(),
                port: 4101,
                ssh_node_port: 32222,
                claim_token: "claim-token".to_string(),
                created_at: "2026-04-09T00:00:00Z".to_string(),
            })
            .unwrap();

        let error = control_plane
            .runtime_upload_files(&RuntimeUploadFilesInput {
                machine_id: "paul-finite-2".to_string(),
                destination_relpath: Some("uploads".to_string()),
                files: vec![RuntimeUploadFileInput {
                    temp_path: "/definitely/missing".to_string(),
                    name: "x.txt".to_string(),
                    size: 1,
                }],
            })
            .unwrap_err();
        assert!(error.to_string().contains("upload temp file missing"));
    }
}
