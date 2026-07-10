{
  description = "Finite monorepo development environment";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-25.11";
    # Kata moves quickly and the 25.11 package is materially behind. Keep the
    # host OS stable while pinning the microVM runtime toolchain independently.
    nixpkgs-kata.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    disko.url = "github:nix-community/disko";
    disko.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      nixpkgs-kata,
      flake-utils,
      disko,
      ...
    }:
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
            just
            nodejs_24
            openssl
            postgresql_16
            pkg-config
            process-compose
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
      packages.x86_64-linux = import ./infra/nixos/packages.nix {
        pkgs = import nixpkgs { system = "x86_64-linux"; };
        src = self;
      };

      # The single app server. Deploying a release IS pinning this flake:
      #   nixos-rebuild switch --target-host root@finite-lat-1 \
      #     --flake github:finitecomputer/finite-mono/<tag-or-rev>#finite-lat-1
      # See infra/nixos/README.md and finite-fable/single-server-plan.md.
      nixosConfigurations.finite-lat-1 = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = {
          finitePackages = self.packages.x86_64-linux;
          kataPackages = import nixpkgs-kata { system = "x86_64-linux"; };
        };
        modules = [
          disko.nixosModules.disko
          ./infra/nixos/hosts/finite-lat-1
        ];
      };
    };
}
