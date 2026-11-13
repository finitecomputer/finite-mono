# Apply bounded Finite patches to the sealed Python environment, so the
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
            cp ${../../finite-agentd/integrations/hermes/finite_dashboard_reads.py} "$site/hermes_cli/finite_dashboard_reads.py"
            cp ${../../finite-agentd/integrations/hermes/finite_inference.py} "$site/hermes_cli/finite_inference.py"

            test -L "$site/gateway"
            cp -RL "$site/gateway" "$site/gateway-patched"
            rm "$site/gateway"
            mv "$site/gateway-patched" "$site/gateway"
            chmod -R u+w "$site/gateway"
            ${pkgs.patch}/bin/patch --fuzz=0 -d "$site" -p1 < ${./patches/hermes-stop-generation.patch}
            test -L "$site/agent"
            cp -RL "$site/agent" "$site/agent-patched"
            rm "$site/agent"
            mv "$site/agent-patched" "$site/agent"
            chmod -R u+w "$site/agent"
            ${pkgs.patch}/bin/patch --fuzz=0 -d "$site" -p1 < ${./patches/hermes-finite-inference.patch}
            rm -f "$site/gateway/__pycache__/run."*.pyc
            rm -f "$site/gateway/__pycache__/slash_commands."*.pyc "$site/hermes_cli/__pycache__/runtime_provider."*.pyc
            rm -f "$site/agent/__pycache__/auxiliary_client."*.pyc
          '';
        });
      }
    else
      result;
}
