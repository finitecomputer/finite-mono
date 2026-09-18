# Apply the native read-contract patch to the sealed Python environment, so the
# Hermes CLI wrapper and exported runtime Python use the same implementation.
# No dependency/version change, PYTHONPATH overlay, or mutable production patch.
{
  pkgs,
  upstream,
  source,
}:
upstream.override {
  callPackage =
    path: args:
    let
      result = pkgs.callPackage path args;
    in
    if toString path == "${source}/nix/python.nix" then
      result
      // {
        venv = result.venv.overrideAttrs (old: {
          postInstall = (old.postInstall or "") + ''
            site="$out/${pkgs.python312.sitePackages}"
            test -L "$site/hermes_cli"
            cp -RL "$site/hermes_cli" "$site/hermes_cli-patched"
            rm "$site/hermes_cli"
            mv "$site/hermes_cli-patched" "$site/hermes_cli"
            chmod -R u+w "$site/hermes_cli"
            ${pkgs.patch}/bin/patch --fuzz=0 -d "$site" -p1 < ${./patches/hermes-skills-inventory.patch}
            rm -f "$site/hermes_cli/web_routers/__pycache__/skills."*.pyc
          '';
        });
      }
    else
      result;
}
