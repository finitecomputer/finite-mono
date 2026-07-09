# Offsite borg backups — every stateful path on the box, daily, pruned.
# The new box is born backed up (single-server-plan.md watch-list item 5;
# lat2/smoke had NO working backup story at capture).
#
# TODO: offsite target decision. The repo URL below is a placeholder.
# Candidates: lat2's /data (1.8T, empty, becomes the CI box — borg over ssh),
# or the legacy fleet's borg target (clawland's box1_borg_backup.sh already
# pushes there). Off-box is the requirement; /data on THIS host does not count.
{ ... }:
{
  services.borgbackup.jobs."finite-offsite" = {
    paths = [
      "/var/lib/finite-sites" # sites data (static user, real path)
      "/var/lib/private/finite-chat" # chat sqlite (DynamicUser real path;
      # /var/lib/finite-chat is only a symlink borg would store as a symlink)
      "/var/lib/private/finitebrain" # brain sqlite (DynamicUser real path)
      "/data/backups/postgres" # timestamped pg_dumps (modules/postgres.nix)
      "/etc/finite-saas" # sites.env + Cloudflare Origin CA cert pair
    ];
    # TODO: replace with the real offsite repo once the target is decided.
    repo = "ssh://borg@TODO-offsite-target/./finite-lat-1";
    encryption = {
      mode = "repokey-blake2";
      # /etc/finite/borg.env (root:root 0600), NAMES only:
      #   BORG_PASSPHRASE   (generated at bootstrap; ALSO store it in the
      #                      team password manager — a passphrase that only
      #                      exists on the box it protects is not a backup)
      passCommand = "sh -c '. /etc/finite/borg.env && printf %s \"$BORG_PASSPHRASE\"'";
    };
    # TODO: generate this ssh key at bootstrap and authorize it (append-only)
    # on the offsite target.
    environment.BORG_RSH = "ssh -i /etc/finite/borg_ed25519";
    compression = "auto,zstd";
    startAt = "daily";
    prune.keep = {
      daily = 7;
      weekly = 4;
      monthly = 6;
    };
  };
}
