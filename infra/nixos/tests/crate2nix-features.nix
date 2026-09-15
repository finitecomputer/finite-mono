# Regression for https://github.com/nix-community/crate2nix/issues/258.
# Activating dep:tracing must not activate a separately named tracing feature.
{ pkgs, sourceRoot }:
let
  generated = import (sourceRoot + "/Cargo.nix") { inherit pkgs; };
  resolve =
    featureMap: selected:
    (generated.internal.mergePackageFeatures {
      packageId = "finitesitesd";
      features = selected;
      target = generated.internal.makeDefaultTarget pkgs.stdenv.hostPlatform;
      crateConfigs = {
        finitesitesd = {
          features = featureMap;
          dependencies = [
            {
              name = "tracing";
              packageId = "tracing";
              optional = true;
            }
          ];
        };
        tracing = {
          features.std = [ ];
        };
      };
    }).finitesitesd;
  namespaced = {
    client = [ "dep:tracing" ];
    tracing = [ "dep:tracing" ];
  };
in
assert !(builtins.elem "tracing" (resolve namespaced [ "client" ]));
assert builtins.elem "tracing" (resolve namespaced [ "tracing" ]);
assert builtins.elem "tracing" (resolve { client = [ "tracing/std" ]; } [ "client" ]);
true
