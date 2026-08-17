{
  description = "Finite monorepo development environment";

  nixConfig = {
    extra-substituters = [
      "https://finite.cachix.org"
    ];
    extra-trusted-public-keys = [
      "finite.cachix.org-1:Sg/y/5ax+IxMrPXS4moFro6YFdqa+a2gzDYAesRcVsk="
    ];
  };

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-25.11";
    # Hermes Agent's PyPI channel was retired in v0.20.0. Keep every repo-owned
    # Hermes runtime path on the upstream Nix package instead of ad hoc archives.
    hermes-nixpkgs.url = "github:NixOS/nixpkgs/0954f7ee2f6bb3dc7d4e3d0d8bcb8fd4bde4cfc5";
    hermes-agent.url = "github:NousResearch/hermes-agent/v2026.8.3";
    hermes-agent.inputs.nixpkgs.follows = "hermes-nixpkgs";
    # finite-lat-3 qualified this NixOS 26.05 platform pin. finite-lat-1 uses
    # the same pin for its platform-only upgrade while retaining its existing
    # disk layout and stateVersion.
    nixpkgs-lat3.url = "github:nixos/nixpkgs/nixos-26.05";
    # Kata moves quickly and the stable package was materially behind when this
    # pin was established. Keep the host OS on a qualified release while
    # pinning the microVM runtime toolchain independently.
    nixpkgs-kata.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    disko.url = "github:nix-community/disko";
    disko.inputs.nixpkgs.follows = "nixpkgs";
    sops-nix.url = "github:Mic92/sops-nix";
    sops-nix.inputs.nixpkgs.follows = "nixpkgs-lat3";

    # Exact installer sources for finite-lat-3. nixos-anywhere's default kexec
    # image is 25.11, so the install always supplies the same-pin tarball built
    # from nixos-images' module below.
    nixos-anywhere.url = "github:nix-community/nixos-anywhere/7239104f1a38546b999cd817658407d80f56e7db";
    nixos-anywhere.inputs.nixpkgs.follows = "nixpkgs-lat3";
    nixos-anywhere.inputs.disko.follows = "disko";
    nixos-anywhere.inputs.nixos-stable.follows = "nixpkgs-lat3";
    nixos-anywhere.inputs.nixos-images.follows = "nixos-images";

    nixos-images.url = "github:nix-community/nixos-images/7ab0da96208ca12907991be63c14e60008c5664b";
    nixos-images.inputs.nixos-stable.follows = "nixpkgs-lat3";
    nixos-images.inputs.nixos-unstable.follows = "nixpkgs-lat3";
  };

  outputs =
    {
      self,
      nixpkgs,
      hermes-nixpkgs,
      hermes-agent,
      nixpkgs-lat3,
      nixpkgs-kata,
      flake-utils,
      rust-overlay,
      disko,
      sops-nix,
      nixos-anywhere,
      nixos-images,
      ...
    }:
    let
      finitePackagesLinux = import ./infra/nixos/packages.nix {
        pkgs = import nixpkgs { system = "x86_64-linux"; };
        sourceRoot = ./.;
      };
      kataPackagesLinux = import nixpkgs-kata { system = "x86_64-linux"; };
      sourceRevision =
        if self ? rev then
          self.rev
        else if self ? dirtyRev then
          self.dirtyRev
        else
          null;
      revisionModule = {
        system.configurationRevision = sourceRevision;
      };
      runnerSpecialArgs = {
        finitePackages = finitePackagesLinux;
        kataPackages = kataPackagesLinux;
      };
      lat3Modules = [
        disko.nixosModules.disko
        sops-nix.nixosModules.sops
        revisionModule
        ./infra/nixos/modules/secrets.nix
        ./infra/nixos/hosts/finite-lat-3
      ];

      # Evaluate stock mirrored GRUB separately so the final host can wrap the
      # generated installer with a fail-before-write ESP identity guard.
      lat3Unguarded = nixpkgs-lat3.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = runnerSpecialArgs;
        modules = lat3Modules;
      };

      lat3 = nixpkgs-lat3.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = runnerSpecialArgs // {
          unguardedInstallBootLoader = lat3Unguarded.config.system.build.installBootLoader;
        };
        modules = lat3Modules ++ [ ./infra/nixos/hosts/finite-lat-3/esp-guard.nix ];
      };

      lat3Kexec = nixpkgs-lat3.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          nixos-images.nixosModules.kexec-installer
          nixos-images.nixosModules.noninteractive
          {
            networking.hostName = "finite-lat-3-installer";
            system.kexec-installer.name = "finite-lat-3-nixos-26.05-kexec";
            system.stateVersion = "26.05";
          }
        ];
      };

      hermesPackagesFor =
        system:
        if builtins.hasAttr system hermes-agent.packages then
          let
            # Same pin as hermes-agent so toolchain ELFs share that glibc.
            hermesPkgs = import hermes-nixpkgs { inherit system; };
            hermesAgentPackage = hermes-agent.packages.${system}.default;
            hermesAgentMinimal = hermes-agent.packages.${system}.minimal;
          in
          {
            hermes-agent = hermesAgentPackage;
            hermes-agent-runtime = hermesAgentPackage;
            hermes-agent-runtime-python = hermesAgentPackage.hermesVenv;
            hermes-agent-minimal = hermesAgentMinimal;
            hermes-agent-minimal-runtime = hermesAgentMinimal.hermesVenv;
            hermes-agent-python = hermesAgentMinimal.hermesVenv;
            agent-runtime-toolchains = hermesPkgs.callPackage ./finitecomputer-v2/deploy/finite-computer/images/agent-runtime-toolchains.nix {
              hermesAgent = hermesAgentPackage;
            };
          }
        else
          { };

      systemOutputs = flake-utils.lib.eachDefaultSystem (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          finitePackages = import ./infra/nixos/packages.nix {
            pkgs = import nixpkgs { inherit system; };
            sourceRoot = ./.;
          };
          gcxCli = (import nixpkgs-lat3 { inherit system; }).gcx;
          # Litestream comes from the lat1 platform pin: 25.11's litestream is
          # 0.3.x and marked insecure, and the restore drill must use the same
          # 0.5 config format the host runs (modules/finite-litestream.nix).
          litestreamCli = (import nixpkgs-lat3 { inherit system; }).litestream;
          rustVersion = "1.93.1";
          # Keep this in sync with the CI Rust workspace pin so cached Cargo
          # artifacts are reusable between clippy and Nix-shell test commands.
          rustToolchain = pkgs.rust-bin.stable.${rustVersion}.default.override {
            extensions = [
              "clippy"
              "rust-analyzer"
              "rust-src"
              "rustfmt"
            ];
            targets = pkgs.lib.optionals pkgs.stdenv.isDarwin [
              "aarch64-apple-ios"
              "aarch64-apple-ios-sim"
            ];
          };
          rustCiToolchain = pkgs.rust-bin.stable.${rustVersion}.default;
          rustBasePackages = with pkgs; [
            curl
            git
            jq
            just
            openssl
            pkg-config
            postgresql_16
            process-compose
            protobuf
            python3
          ];
          rustCiPackages = rustBasePackages ++ [ rustCiToolchain ];
          # CI's devfinity smoke starts the dashboard dev server, but it does not
          # run browser tests or need local editor/tooling extras from the default shell.
          devfinityCiPackages =
            rustBasePackages
            ++ (with pkgs; [
              nodejs_24
              pnpm
              rustCiToolchain
            ]);
          hermesSupported = builtins.hasAttr system hermes-agent.packages;
        in
        {
          packages = (hermesPackagesFor system) // finitePackages;

          devShells = {
            default = pkgs.mkShell {
              packages =
                rustBasePackages
                ++ [
                  pkgs.age
                  gcxCli
                  litestreamCli
                  pkgs.sops
                ]
                ++ (with pkgs; [
                  nodejs_24
                  pnpm
                  rsync
                  sqlite
                  xxd
                  rustToolchain
                ])
                ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.xcodegen ]
                ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.chromium ];

              RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
            };

            rust-ci = pkgs.mkShell {
              packages = rustCiPackages;
            };

            devfinity-ci = pkgs.mkShell {
              packages = devfinityCiPackages;
            };
          }
          // pkgs.lib.optionalAttrs hermesSupported (
            let
              hermesAgentRuntime = hermes-agent.packages.${system}.default;
              hermesAgentRuntimePython = hermesAgentRuntime.hermesVenv;
              hermesBridgePkgs = import hermes-nixpkgs { inherit system; };
            in
            {
              hermes-bridge-ci = pkgs.mkShell {
                packages = [
                  hermesAgentRuntime
                  hermesAgentRuntimePython
                  hermesBridgePkgs.basedpyright
                  hermesBridgePkgs.ruff
                ];

                HERMES_AGENT_RUNTIME_PYTHON = "${hermesAgentRuntimePython}/bin/python3";
                HERMES_AGENT_PYTHON = "${hermesAgentRuntimePython}/bin/python3";
              };
            }
          );

          formatter = pkgs.nixfmt-rfc-style;
        }
      );
    in
    systemOutputs
    // {
      packages = systemOutputs.packages // {
        # Server binaries + CLIs built by nix from this workspace (built by CI /
        # Depot-backed runners; eval-only on darwin). See infra/nixos/packages.nix.
        x86_64-linux =
          systemOutputs.packages.x86_64-linux
          // finitePackagesLinux
          // {
            finite-lat-3-system = lat3.config.system.build.toplevel;
            finite-lat-3-disko = lat3.config.system.build.diskoScript;
            finite-lat-3-kexec = lat3Kexec.config.system.build.kexecInstallerTarball;
            finite-lat-3-nixos-anywhere = nixos-anywhere.packages.x86_64-linux.nixos-anywhere;
          };
      };

      # The single app server. Deploying a release IS pinning this flake:
      #   nixos-rebuild switch --target-host root@finite-lat-1 \
      #     --flake github:finitecomputer/finite-mono/<tag-or-rev>#finite-lat-1
      # See infra/nixos/README.md and finite-fable/single-server-plan.md.
      nixosConfigurations.finite-lat-1 = nixpkgs-lat3.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = runnerSpecialArgs;
        modules = [
          disko.nixosModules.disko
          sops-nix.nixosModules.sops
          revisionModule
          ./infra/nixos/modules/secrets.nix
          ./infra/nixos/hosts/finite-lat-1
        ];
      };

      # The qualified blank-slate host carries the Standard Runner accepting
      # new creation with its host-configured hard sandbox limit.
      nixosConfigurations.finite-lat-3 = lat3;
    };
}
