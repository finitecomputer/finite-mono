# finitesitesd — Finite Sites registry, publishing API, Git smart HTTP, and
# site serving for *.finite.chat / *.docs.finite.chat on 127.0.0.1:8787.
# Ported from lat2's finite-saas-sites.service (byte-identical flags except
# --app-runner, see below). Data: /var/lib/finite-sites (restored from lat2
# at cutover).
#
# ############################################################################
# ## KATA ISOLATION TODO                                                    ##
# ##                                                                        ##
# ## lat2 ran `--app-runner kata`: tier-2 tenant apps as Kata Containers    ##
# ## 3.31.0 microVMs on cloud-hypervisor ([hypervisor.clh] in               ##
# ## /etc/kata-containers/configuration.toml), driven via `sudo nerdctl`    ##
# ## (2.3.1) + containerd, with a systemd drop-in                           ##
# ## (finite-saas-sites-kata.conf) relaxing NoNewPrivileges/ProtectSystem   ##
# ## so the daemon could spawn sudo-nerdctl, plus a sudoers file gating     ##
# ## finite-sites to nerdctl alone (all captured in                         ##
# ## infra/hosts/lat2/systemd/).                                            ##
# ##                                                                        ##
# ## This module deliberately ships WITHOUT Kata (--app-runner none): per   ##
# ## single-server-plan.md, Kata must not block the cutover. That means     ##
# ## tier-2 apps DO NOT RUN until this is resolved — static sites, the      ##
# ## registry, publishing, and git all work. WEAKENED-ISOLATION/FEATURE     ##
# ## GAP, explicit and tracked.                                             ##
# ##                                                                        ##
# ## TODO(kata): pick one and implement:                                    ##
# ##   Plan A — package kata-runtime 3.31.x + cloud-hypervisor + containerd ##
# ##            + nerdctl on NixOS and port the lat2 drop-in/sudoers        ##
# ##            (restores exact parity).                                    ##
# ##   Plan B — microvm.nix (nix-native microVMs; needs a finitesitesd      ##
# ##            app-runner backend).                                        ##
# ##   Interim, if tier-2 apps are needed before A/B: --app-runner systemd  ##
# ##   + the finite-app@.service template + polkit rule from                ##
# ##   infra/hosts/lat2/systemd/ (process isolation only).                  ##
# ############################################################################
{ finitePackages, ... }:
{
  users.users.finite-sites = {
    isSystemUser = true;
    group = "finite-sites";
  };
  users.groups.finite-sites = { };

  systemd.services.finite-saas-sites = {
    description = "Finite Sites (registry, publishing API, site serving)";
    wants = [ "network-online.target" ];
    after = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];

    serviceConfig = {
      User = "finite-sites";
      Group = "finite-sites";
      # Same flags as lat2 except --app-runner (see KATA ISOLATION TODO).
      ExecStart = ''
        ${finitePackages.finitesitesd}/bin/finitesitesd serve \
          --data /var/lib/finite-sites \
          --listen 127.0.0.1:8787 \
          --base-domain finite.chat \
          --document-base-domain docs.finite.chat \
          --api-url https://api.finite.chat \
          --git-url https://git.finite.chat \
          --site-scheme https \
          --site-port none \
          --mailer resend \
          --mail-from "Finite Sites <links@finite.chat>" \
          --app-runner none
      '';
      # Operator-created, root:root 0640 (or readable by finite-sites).
      # Variable NAMES only (values from lat2's /etc/finite-saas/sites.env):
      #   RESEND_API_KEY
      #   FINITE_IDENTITY_AUTHORITY  (optional; not set live on lat2)
      EnvironmentFile = "/etc/finite-saas/sites.env";
      Restart = "on-failure";
      RestartSec = 2;
      # Full lat2 hardening applies — no kata drop-in to relax it.
      StateDirectory = "finite-sites";
      ProtectSystem = "strict";
      ReadWritePaths = [ "/var/lib/finite-sites" ];
      ProtectHome = true;
      PrivateTmp = true;
      NoNewPrivileges = true;
      ProtectKernelTunables = true;
      ProtectControlGroups = true;
      RestrictSUIDSGID = true;
    };
  };
}
