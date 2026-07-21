{
  description = "Finite monorepo development environment";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-25.11";
    # finite-lat-3 is the blank-slate NixOS 26.05 qualification host. Keep its
    # platform pin independent until the lat1 closure upgrade is accepted.
    nixpkgs-lat3.url = "github:nixos/nixpkgs/nixos-26.05";
    # Kata moves quickly and the 25.11 package is materially behind. Keep the
    # host OS stable while pinning the microVM runtime toolchain independently.
    nixpkgs-kata.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    disko.url = "github:nix-community/disko";
    disko.inputs.nixpkgs.follows = "nixpkgs";

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
      nixpkgs-lat3,
      nixpkgs-kata,
      flake-utils,
      disko,
      nixos-anywhere,
      nixos-images,
      ...
    }:
    let
      finitePackagesLinux = import ./infra/nixos/packages.nix {
        pkgs = import nixpkgs { system = "x86_64-linux"; };
        src = self;
      };
      kataPackagesLinux = import nixpkgs-kata { system = "x86_64-linux"; };
      runnerSpecialArgs = {
        finitePackages = finitePackagesLinux;
        kataPackages = kataPackagesLinux;
      };
      lat3Modules = [
        disko.nixosModules.disko
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
    in
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            clippy
            curl
            git
            just
            nodejs_24
            openssl
            postgresql_16
            pkg-config
            process-compose
            python3
            rsync
            rust-analyzer
            rustPlatform.rustLibSrc
            rustc
            rustfmt
          ];

          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };

        formatter = pkgs.nixfmt-rfc-style;
      }
    )
    // {
      # Server binaries + CLIs built by nix from this workspace (built by CI /
      # the lat2 runner; eval-only on darwin). See infra/nixos/packages.nix.
      packages.x86_64-linux = finitePackagesLinux // {
        finite-lat-3-system = lat3.config.system.build.toplevel;
        finite-lat-3-disko = lat3.config.system.build.diskoScript;
        finite-lat-3-kexec = lat3Kexec.config.system.build.kexecInstallerTarball;
        finite-lat-3-nixos-anywhere = nixos-anywhere.packages.x86_64-linux.nixos-anywhere;
      };

      # The single app server. Deploying a release IS pinning this flake:
      #   nixos-rebuild switch --target-host root@finite-lat-1 \
      #     --flake github:finitecomputer/finite-mono/<tag-or-rev>#finite-lat-1
      # See infra/nixos/README.md and finite-fable/single-server-plan.md.
      nixosConfigurations.finite-lat-1 = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = runnerSpecialArgs;
        modules = [
          disko.nixosModules.disko
          ./infra/nixos/hosts/finite-lat-1
        ];
      };

      # The qualified blank-slate host carries the Standard Runner accepting
      # new creation with its host-configured hard sandbox limit.
      nixosConfigurations.finite-lat-3 = lat3;
    };
}
