# finite-lat-2's mover (../../modules/finite-mover.nix) comes from the host
# revision, as the Runner hosts' finite-mover-host does; Core and every
# other application stay on the lat2-app-source pin (flake.nix). A
# mover-only change is then a host-only deploy: no application executable
# changes, so no application restarts.
#
# The rule: the mover talks to the PINNED Core. It reads Core's database
# through core-role.sql's columns and runs four of Core's CLI subcommands
# (finite-mover's core_cli.rs). A mover change that needs a newer Core (a
# column, a subcommand, a flag, its output or its behaviour) still needs
# the pin advanced to that Core, before or with the mover.
#
# Building this host checks the columns: it applies the mover's
# core-role.sql to the pinned Core's migrations, as finite-mover-postgres
# does on the box, and fails if a grant names something that Core lacks.
# Nothing checks the subcommands or behaviour.
{
  mover,
  coreMigrations,
}:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  postgresql = config.services.postgresql.finalPackage;
  migrations = lib.sort lib.lessThan (
    builtins.filter (name: builtins.match "[0-9]{4}_.*[.]sql" name != null) (
      builtins.attrNames (builtins.readDir coreMigrations)
    )
  );
  psql = "psql --quiet --dbname=finite_core --set=ON_ERROR_STOP=1";
  grantsFitPinnedCore =
    pkgs.runCommand "finite-mover-grants-fit-pinned-core"
      {
        nativeBuildInputs = [ postgresql ];
      }
      ''
        export PGDATA="$TMPDIR/data" PGHOST="$TMPDIR" PGUSER=postgres
        export PGOPTIONS="-c client_min_messages=warning"
        initdb --auth=trust --username=postgres --no-sync >/dev/null
        pg_ctl start --wait --log="$TMPDIR/postgres.log" \
          -o "-c listen_addresses= -c unix_socket_directories=$TMPDIR -c fsync=off" >/dev/null \
          || { cat "$TMPDIR/postgres.log" >&2; exit 1; }
        createdb finite_core
        ${lib.concatMapStrings (name: ''
          ${psql} --file=${coreMigrations}/${name}
        '') migrations}
        ${psql} --command='CREATE ROLE finite_mover LOGIN'
        ${psql} --single-transaction --file=${mover}/share/finite-mover/core-role.sql
        pg_ctl stop --wait >/dev/null
        touch "$out"
      '';
in
{
  finite.mover.package = mover;
  system.extraDependencies = [ grantsFitPinnedCore ];
}
