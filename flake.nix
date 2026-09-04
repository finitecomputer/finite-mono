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
    crane.url = "github:ipetkov/crane/v0.23.4";
    # Hermes Agent's PyPI channel was retired in v0.20.0. Keep every repo-owned
    # Hermes runtime path on the upstream Nix package instead of ad hoc archives.
    hermes-nixpkgs.url = "github:NixOS/nixpkgs/0954f7ee2f6bb3dc7d4e3d0d8bcb8fd4bde4cfc5";
    # Fetch Hermes over git smart HTTP (rev-pinned), never a GitHub archive:
    # `github:` inputs resolve to api.github.com/repos/.../tarball/<rev> and
    # the codeload archive URL (github.com/.../archive/<rev>.tar.gz) is the
    # other front door to the same archive service. Both share GitHub's
    # per-IP secondary rate limit, which shared CI runner egress IPs trip
    # (HTTP 429) — chronically, even after the move to the archive endpoint.
    # The smart-HTTP git fetch (`git+https`) is not behind that limit. Same
    # upstream flake, same tag/rev, identical source tree (the lock's narHash
    # is unchanged by the switch). `shallow=1` keeps the fresh-runner clone
    # to the pinned commit (~60 MB) instead of full history (~500 MB); it
    # locks to the same rev/narHash. Bumps edit the rev here and re-lock.
    # Current pin: v2026.8.3 (rev 3c27eb62).
    hermes-agent.url = "git+https://github.com/NousResearch/hermes-agent?rev=3c27eb6234bf91b8ceee9e9071591b31e9b148cb&shallow=1";
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
      crane,
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
      finitePackagePkgsLinux = import nixpkgs { system = "x86_64-linux"; };
      finitePackagesLinux = import ./infra/nixos/packages.nix {
        pkgs = finitePackagePkgsLinux;
        craneLib = crane.mkLib finitePackagePkgsLinux;
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

      # finite-lat-2 is the emergency replacement app-plane host (ADR 0007):
      # lat1's service stack on the lat3 storage chassis, no Agent Runner.
      lat2Modules = [
        disko.nixosModules.disko
        sops-nix.nixosModules.sops
        revisionModule
        ./infra/nixos/modules/secrets.nix
        ./infra/nixos/hosts/finite-lat-2
      ];

      lat2Unguarded = nixpkgs-lat3.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = runnerSpecialArgs;
        modules = lat2Modules;
      };

      lat2 = nixpkgs-lat3.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = runnerSpecialArgs // {
          unguardedInstallBootLoader = lat2Unguarded.config.system.build.installBootLoader;
        };
        modules = lat2Modules ++ [ ./infra/nixos/hosts/finite-lat-2/esp-guard.nix ];
      };

      lat2Kexec = nixpkgs-lat3.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          nixos-images.nixosModules.kexec-installer
          nixos-images.nixosModules.noninteractive
          {
            networking.hostName = "finite-lat-2-installer";
            system.kexec-installer.name = "finite-lat-2-nixos-26.05-kexec";
            system.stateVersion = "26.05";
          }
        ];
      };

      # finite-lat-4 is the third storage-qualified Runner host (ADR 0007
      # model, following the lat2 rejoin in PR #715). It mirrors the lat3
      # module stack, including the ESP-guard double eval.
      lat4Modules = [
        disko.nixosModules.disko
        sops-nix.nixosModules.sops
        revisionModule
        ./infra/nixos/modules/secrets.nix
        ./infra/nixos/hosts/finite-lat-4
      ];

      lat4Unguarded = nixpkgs-lat3.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = runnerSpecialArgs;
        modules = lat4Modules;
      };

      lat4 = nixpkgs-lat3.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = runnerSpecialArgs // {
          unguardedInstallBootLoader = lat4Unguarded.config.system.build.installBootLoader;
        };
        modules = lat4Modules ++ [ ./infra/nixos/hosts/finite-lat-4/esp-guard.nix ];
      };

      lat4Kexec = nixpkgs-lat3.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          nixos-images.nixosModules.kexec-installer
          nixos-images.nixosModules.noninteractive
          {
            networking.hostName = "finite-lat-4-installer";
            system.kexec-installer.name = "finite-lat-4-nixos-26.05-kexec";
            system.stateVersion = "26.05";
          }
        ];
      };

      # Dedicated static-only Finite Sites v2 validation host (ADR 0028).
      sitesV2 = nixpkgs-lat3.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = runnerSpecialArgs;
        modules = [
          revisionModule
          ./infra/nixos/hosts/finite-sites-v2
        ];
      };

      monitoring = nixpkgs-lat3.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          revisionModule
          ./infra/nixos/hosts/finite-monitoring
        ];
      };

      # Hermes attrs are deliberately lazy: nothing below forces the hermes
      # inputs until a consumer actually requests a hermes package (or enters
      # the hermes-bridge-ci shell), so `nix develop .#rust-ci`-style
      # evaluations never download hermes — hermes fetches were the recurring
      # CI 429 blocker on shared runner IPs. The trade: requesting a hermes
      # attr on a system upstream does not package (x86_64-darwin) errors at
      # that point instead of the attribute simply being absent.
      hermesPackagesFor =
        system:
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
          agent-runtime-toolchains =
            hermesPkgs.callPackage
              ./finitecomputer-v2/deploy/finite-computer/images/agent-runtime-toolchains.nix
              {
                hermesAgent = hermesAgentPackage;
              };
        };

      systemOutputs = flake-utils.lib.eachDefaultSystem (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          finitePackagePkgs = import nixpkgs { inherit system; };
          # The repo-wide Python formatter/linter pin. Deliberately from the
          # hermes-nixpkgs pin (ruff 0.15.x, the version the tree is
          # formatted with and hermes-bridge-ci checks with) so local, CI,
          # and editor invocations can never disagree. Invoke only via
          # `just fmt-py` / `just lint-py`; format output differs between
          # ruff versions and ad-hoc copies are how formatting gates go red.
          pyToolPkgs = import hermes-nixpkgs { inherit system; };
          finitePackages = import ./infra/nixos/packages.nix {
            pkgs = finitePackagePkgs;
            craneLib = crane.mkLib finitePackagePkgs;
            sourceRoot = ./.;
          };
          gcxCli = (import nixpkgs-lat3 { inherit system; }).gcx;
          # Litestream comes from the lat1 platform pin: 25.11's litestream is
          # 0.3.x and marked insecure, and the restore drill must use the same
          # 0.5 config format the host runs (modules/finite-litestream.nix).
          litestreamCli = (import nixpkgs-lat3 { inherit system; }).litestream;
          # The Rust pin lives in ./rust-toolchain.toml (single source for
          # rustup on dev hosts, the CI workflows, and these Nix shells).
          # Cached Cargo artifacts stay reusable between clippy, Nix-shell
          # test commands, and image builds because they all read the same
          # file.
          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          rustCiToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
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
                  pyToolPkgs.ruff
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

            # Entering this shell is one of the few things that forces the
            # hermes inputs (see hermesPackagesFor) — the shells above stay
            # hermes-free so unrelated CI jobs never fetch hermes.
            hermes-bridge-ci =
              let
                hermesAgentRuntime = hermes-agent.packages.${system}.default;
                hermesAgentRuntimePython = hermesAgentRuntime.hermesVenv;
              in
              pkgs.mkShell {
                packages = [
                  hermesAgentRuntime
                  hermesAgentRuntimePython
                  pyToolPkgs.basedpyright
                  pyToolPkgs.ruff
                ];

                HERMES_AGENT_RUNTIME_PYTHON = "${hermesAgentRuntimePython}/bin/python3";
                HERMES_AGENT_PYTHON = "${hermesAgentRuntimePython}/bin/python3";
              };
          };

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
            finite-lat-2-system = lat2.config.system.build.toplevel;
            # Partitioning uses the plain (unguarded) disko script: the ESP
            # guard lives in the guarded system closure's bootloader
            # installer and runs at switch time, not at partition time.
            # Requesting the guarded eval's disko here pulls both disko
            # derivations into the build and breaks out-link determinism.
            finite-lat-2-disko = lat2Unguarded.config.system.build.diskoScript;
            finite-lat-2-kexec = lat2Kexec.config.system.build.kexecInstallerTarball;
            finite-lat-2-nixos-anywhere = nixos-anywhere.packages.x86_64-linux.nixos-anywhere;
            finite-lat-4-system = lat4.config.system.build.toplevel;
            finite-lat-4-disko = lat4.config.system.build.diskoScript;
            finite-lat-4-kexec = lat4Kexec.config.system.build.kexecInstallerTarball;
            finite-lat-4-nixos-anywhere = nixos-anywhere.packages.x86_64-linux.nixos-anywhere;
            finite-sites-v2-system = sitesV2.config.system.build.toplevel;
            finite-monitoring-system = monitoring.config.system.build.toplevel;
          };
      };

      # Retired app-plane host retained only for historical contract checks and
      # forensic reference. ADR 0007 moved production to finite-lat-2; do not
      # build or activate this as a production target.
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

      # The replacement app server. Installed and brought up only through
      # infra/runbooks/lat2-replacement-cutover.md (ADR 0007); it boots in
      # import mode and goes live at Gate E.
      nixosConfigurations.finite-lat-2 = lat2;

      # The third storage-qualified Runner host (ADR 0007 model). Installed
      # and admitted only through
      # infra/runbooks/lat4-nixos-runner-install.md; it starts drained.
      nixosConfigurations.finite-lat-4 = lat4;

      # Dedicated static-only Finite Sites v2 validation host (ADR 0028).
      # The current app-plane hosts keep their canonical edge until cutover.
      nixosConfigurations.finite-sites-v2 = sitesV2;

      # Dedicated NixOS Grafana/Prometheus/Loki receiver. This is the hard-cut
      # replacement for the historical monitoring Docker Compose stack.
      nixosConfigurations.finite-monitoring = monitoring;
    };
}
