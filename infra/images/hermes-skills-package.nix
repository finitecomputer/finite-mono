# Apply Finite runtime patches to the sealed Python environment, so the
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
            test -L "$site/agent"
            cp -RL "$site/agent" "$site/agent-patched"
            rm "$site/agent"
            mv "$site/agent-patched" "$site/agent"
            chmod -R u+w "$site/agent"
            ${pkgs.patch}/bin/patch --fuzz=0 -d "$site" -p1 < ${../../finite-agentd/patches/hermes-stream-writer.patch}
            rm -f "$site/agent/__pycache__/chat_completion_helpers."*.pyc
            cp -L "$site/run_agent.py" "$site/run_agent-patched.py"
            rm "$site/run_agent.py"
            mv "$site/run_agent-patched.py" "$site/run_agent.py"
            chmod u+w "$site/run_agent.py"
            ${pkgs.patch}/bin/patch --fuzz=0 -d "$site" -p1 < ${../../finite-agentd/patches/hermes-steer-history.patch}
            rm -f "$site/__pycache__/run_agent."*.pyc
            rm -f "$site/agent/__pycache__/agent_runtime_helpers."*.pyc
          '';
        });
      }
    else
      result;
}
